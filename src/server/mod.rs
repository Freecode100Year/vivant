//! Wires the chat hub to SSH listeners.
//!
//! Security posture:
//! - No shell, no exec, no SFTP/SCP, no port/agent/X11 forwarding: the only
//!   thing an SSH session can reach is the chat UI.
//! - Anonymous login: any username, no password. Ordinary users are let in
//!   at the SSH "none" authentication step, so their client never offers a
//!   public key — keys are linkable identities (github.com/<user>.keys).
//!   Only the admin username goes on to public-key authentication.
//! - Client IP addresses are never logged or stored. Rate limits and bans
//!   use an HMAC of the address (IPv6 grouped by /64) under a random key that
//!   only lives in memory and changes on every restart (unless ban
//!   persistence is switched on, see bans.rs).

mod bans;
mod stream;
mod torexits;

use crate::chat::{ConnectError, Hub, ONION_KEY};
use crate::log_info;
use crate::ui::{self, Input, Io};
use russh::keys::ssh_key::{self, LineEnding, PublicKey};
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId, ChannelMsg, MethodKind, MethodSet, Preferred, SshId, cipher, kex, mac};
use std::borrow::Cow;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use stream::{ConnStream, Kill};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

/// Configures the SSH front-end.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub addr: String,
    pub host_key_path: PathBuf,
    /// authorized_keys-format file; optional
    pub admin_keys_path: Option<PathBuf>,
    /// disconnect after this long without traffic (zero = never)
    pub idle_timeout: Duration,
    /// raw TCP connections per address, incl. unauthenticated (default 8)
    pub max_tcp_per_ip: usize,
    /// raw TCP connections in total, incl. unauthenticated (default 2500)
    pub max_tcp: usize,
    /// bytes/s a connection may send us; excess is slowed down (0 = off)
    pub read_rate: usize,
    pub read_burst: usize,
    /// username that authenticates with an admin key (default "admin")
    pub admin_user: String,
    /// optional second listener for a Tor onion service
    pub onion_addr: String,
    /// TCP connections allowed through the onion listener (default 400)
    pub onion_max_tcp: usize,
    /// optional local listener for tunnels (frp, EasyTier, …): every client
    /// arrives from the tunnel's own address, so they share the onion bucket
    pub relay_addr: String,
    pub ban_file: Option<PathBuf>,
    /// extra listen addresses, e.g. ":443"
    pub alt_addrs: Vec<String>,
    /// fetch Tor's exit list so Tor Browser users share the Tor bucket
    pub tor_exits: bool,
}

/// A connection that hasn't opened a chat session by then is closed. Real
/// clients take well under a second (a few over Tor); port scanners and
/// stalled handshakes would otherwise hold a connection slot for an hour.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

struct Shared {
    hub: Arc<Hub>,
    salt: Vec<u8>,
    admins: Vec<PublicKey>,
    admin_user: String,
    tcp: Mutex<TcpCounts>,
    tcp_max: usize,
    tcp_all: usize,
    onion_max: usize,
    read_rate: usize,
    read_burst: usize,
    tor: torexits::ExitSet,
    config: Arc<russh::server::Config>,
}

#[derive(Default)]
struct TcpCounts {
    per_key: HashMap<String, usize>,
    total: usize,
}

pub struct Server {
    shared: Arc<Shared>,
    opt: Options,
}

impl Server {
    pub fn new(hub: Arc<Hub>, mut opt: Options) -> Result<Server, String> {
        if opt.max_tcp_per_ip == 0 {
            opt.max_tcp_per_ip = 8;
        }
        if opt.max_tcp == 0 {
            opt.max_tcp = 2500;
        }
        if opt.admin_user.is_empty() {
            opt.admin_user = "admin".into();
        }
        if opt.onion_max_tcp == 0 {
            opt.onion_max_tcp = 400;
        }
        let mut salt = vec![0u8; 32];
        crate::chat::random_bytes(&mut salt);
        if let Some(path) = &opt.ban_file {
            let mut saved = bans::read_bans(path).map_err(|e| format!("ban file: {e}"))?;
            let key_path = path.parent().unwrap_or(Path::new(".")).join("ban.key");
            if bans::active(&saved) == 0 {
                // Nothing to carry over: start with a fresh key, so the old
                // one (and any copy of it) can't be used against past bans.
                match std::fs::remove_file(&key_path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("ban key: {e}")),
                }
                saved.clear();
            }
            salt = bans::load_or_create_salt(&key_path).map_err(|e| format!("ban key: {e}"))?;
            log_info!("ban persistence enabled", loaded = hub.load_bans(saved));
        }
        let mut admins = Vec::new();
        if let Some(path) = &opt.admin_keys_path {
            admins = load_authorized_keys(path).map_err(|e| format!("admin keys: {e}"))?;
            log_info!("loaded admin keys", count = admins.len());
        }
        let host_key = load_or_create_host_key(&opt.host_key_path).map_err(|e| format!("host key: {e}"))?;
        let mut methods = vec![MethodKind::None, MethodKind::KeyboardInteractive];
        if !admins.is_empty() {
            methods.push(MethodKind::PublicKey);
        }
        let config = russh::server::Config {
            server_id: SshId::Standard(Cow::Borrowed("SSH-2.0-VIVANT")),
            methods: MethodSet::from(&methods[..]),
            auth_rejection_time: Duration::from_millis(300),
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            inactivity_timeout: (!opt.idle_timeout.is_zero()).then_some(opt.idle_timeout),
            // A keepalive every 30 s keeps NAT mappings and middleboxes from
            // dropping quiet sessions. Unanswered keepalives never end a session
            // (keepalive_max 0): a phone app that the OS suspends on lock screen
            // cannot answer them, but the phone's TCP stack still acknowledges
            // the packets, so the session survives and is there on unlock. A
            // peer that is really gone (network lost, laptop powered off) stops
            // acknowledging; the kernel then gives up retransmitting the
            // keepalives (net.ipv4.tcp_retries2, about 15 minutes) and closes
            // the connection, which frees the slot.
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 0,
            max_auth_attempts: 6,
            nodelay: true,
            preferred: preferred(),
            ..Default::default()
        };
        let shared = Arc::new(Shared {
            hub,
            salt,
            admins,
            admin_user: opt.admin_user.to_lowercase(),
            tcp: Mutex::new(TcpCounts::default()),
            tcp_max: opt.max_tcp_per_ip,
            tcp_all: opt.max_tcp,
            onion_max: opt.onion_max_tcp,
            read_rate: opt.read_rate,
            read_burst: opt.read_burst.max(opt.read_rate),
            tor: torexits::ExitSet::default(),
            config: Arc::new(config),
        });
        Ok(Server { shared, opt })
    }

    /// Serves until `stop` fires, then shuts down gracefully.
    pub async fn run(self, mut stop: watch::Receiver<bool>) -> Result<(), String> {
        let sh = self.shared.clone();
        let main = listen(&self.opt.addr).await.map_err(|e| format!("listen {}: {e}", self.opt.addr))?;
        let mut listeners = vec![(main, false)];
        for a in &self.opt.alt_addrs {
            let l = listen(a).await.map_err(|e| format!("alternate listener {a}: {e}"))?;
            log_info!("alternate listener", addr = a);
            listeners.push((l, false));
        }
        if !self.opt.onion_addr.is_empty() {
            let a = &self.opt.onion_addr;
            let l = listen(a).await.map_err(|e| format!("onion listener: {e}"))?;
            log_info!("onion listener", addr = a);
            listeners.push((l, true));
        }
        if !self.opt.relay_addr.is_empty() {
            let a = &self.opt.relay_addr;
            let l = listen(a).await.map_err(|e| format!("relay listener: {e}"))?;
            log_info!("relay listener", addr = a);
            listeners.push((l, true));
        }
        let (fatal_tx, mut fatal_rx) = mpsc::channel::<String>(1);
        for (l, onion) in listeners {
            tokio::spawn(accept_loop(sh.clone(), l, onion, fatal_tx.clone()));
        }
        if self.opt.tor_exits {
            tokio::spawn(torexits::refresh(sh.tor.clone(), stop.clone()));
        }
        let persist = self.opt.ban_file.clone().map(|p| tokio::spawn(bans::persist(sh.hub.clone(), p, stop.clone())));
        let hub = sh.hub.clone();
        tokio::spawn(async move {
            let mut presence = tokio::time::interval(Duration::from_secs(30));
            let mut janitor = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = presence.tick() => hub.flush_presence(),
                    _ = janitor.tick() => hub.janitor(),
                }
            }
        });
        tokio::select! {
            e = fatal_rx.recv() => return Err(e.unwrap_or_default()),
            _ = stop.wait_for(|s| *s) => {}
        }
        sh.hub.shutdown("Server is restarting — please reconnect in a moment.");
        // give sessions a moment to print the notice and close
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while sh.tcp.lock().unwrap().total > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if let Some(p) = persist {
            let _ = p.await; // final save
        }
        Ok(())
    }
}

/// Binds "host:port"; an empty host (":2222", as in the Go version) means
/// every address, IPv6 and IPv4.
async fn listen(addr: &str) -> std::io::Result<TcpListener> {
    if let Some(port) = addr.strip_prefix(':') {
        let port: u16 = port.parse().map_err(|_| std::io::Error::other("bad port"))?;
        return match TcpListener::bind(("::", port)).await {
            Ok(l) => Ok(l),
            Err(_) => TcpListener::bind(("0.0.0.0", port)).await, // no IPv6 here
        };
    }
    TcpListener::bind(addr).await
}

/// The algorithms offered: no SHA-1 key exchange or MACs, and no
/// finite-field Diffie-Hellman, whose big modular exponentiation is the most
/// CPU a client can make the server spend before it authenticates. Every SSH
/// client from the last decade speaks curve25519.
fn preferred() -> Preferred {
    Preferred {
        kex: Cow::Borrowed(&[
            kex::MLKEM768X25519_SHA256, // post-quantum hybrid
            kex::CURVE25519,
            kex::CURVE25519_PRE_RFC_8731,
            kex::ECDH_SHA2_NISTP256,
            kex::ECDH_SHA2_NISTP384,
            kex::ECDH_SHA2_NISTP521,
            kex::EXTENSION_SUPPORT_AS_SERVER,
            kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
        ]),
        key: Cow::Borrowed(&[ssh_key::Algorithm::Ed25519]),
        cipher: Cow::Borrowed(&[
            cipher::CHACHA20_POLY1305,
            cipher::AES_256_GCM,
            cipher::AES_128_GCM,
            cipher::AES_256_CTR,
            cipher::AES_192_CTR,
            cipher::AES_128_CTR,
        ]),
        mac: Cow::Borrowed(&[mac::HMAC_SHA256_ETM, mac::HMAC_SHA512_ETM, mac::HMAC_SHA256, mac::HMAC_SHA512]),
        compression: Cow::Borrowed(&[russh::compression::NONE]),
        ..Preferred::default()
    }
}

async fn accept_loop(sh: Arc<Shared>, l: TcpListener, onion: bool, fatal: mpsc::Sender<String>) {
    loop {
        let (sock, peer) = match l.accept().await {
            Ok(x) => x,
            Err(e) => {
                // EMFILE and friends: back off instead of spinning
                if matches!(e.raw_os_error(), Some(23) | Some(24)) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                if e.kind() == std::io::ErrorKind::ConnectionAborted {
                    continue;
                }
                let _ = fatal.send(format!("accept: {e}")).await;
                return;
            }
        };
        let key = key_for(&sh, onion, peer.ip());
        let Some(slot) = TcpSlot::take(&sh, &key) else {
            drop(sock); // over the per-address or global cap
            continue;
        };
        tokio::spawn(serve_conn(sh.clone(), sock, key, slot));
    }
}

/// Holds one per-address TCP slot; released on drop.
struct TcpSlot {
    sh: Arc<Shared>,
    key: String,
}

impl TcpSlot {
    fn take(sh: &Arc<Shared>, key: &str) -> Option<TcpSlot> {
        let limit = if key == ONION_KEY { sh.onion_max } else { sh.tcp_max };
        let mut t = sh.tcp.lock().unwrap();
        if t.per_key.get(key).copied().unwrap_or(0) >= limit || t.total >= sh.tcp_all {
            return None;
        }
        *t.per_key.entry(key.to_string()).or_insert(0) += 1;
        t.total += 1;
        Some(TcpSlot { sh: sh.clone(), key: key.to_string() })
    }
}

impl Drop for TcpSlot {
    fn drop(&mut self) {
        let mut t = self.sh.tcp.lock().unwrap();
        t.total -= 1;
        match t.per_key.get_mut(&self.key) {
            Some(n) if *n > 1 => *n -= 1,
            _ => {
                t.per_key.remove(&self.key);
            }
        }
    }
}

/// The rate-limit key for a connection: everyone arriving through the onion
/// listener or from a Tor exit relay shares one key (the server can't tell
/// them apart, which is the point); everyone else gets an HMAC of their
/// address.
fn key_for(sh: &Shared, onion: bool, ip: IpAddr) -> String {
    let ip = ip.to_canonical(); // IPv4 clients on a dual-stack socket arrive as ::ffff:a.b.c.d
    if onion || sh.tor.contains(&ip) {
        return ONION_KEY.to_string();
    }
    addr_key(&sh.salt, ip)
}

/// HMAC-SHA256 of the address bytes (IPv4: 4 bytes; IPv6: the /64 prefix
/// as 16 bytes), first 12 bytes as hex — the same as the Go version, so
/// persisted bans survive the upgrade.
pub fn addr_key(salt: &[u8], ip: IpAddr) -> String {
    let bytes: Vec<u8> = match ip {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            o.to_vec()
        }
    };
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, salt);
    let tag = ring::hmac::sign(&k, &bytes);
    tag.as_ref()[..12].iter().map(|b| format!("{b:02x}")).collect()
}

async fn serve_conn(sh: Arc<Shared>, sock: TcpStream, key: String, slot: TcpSlot) {
    let _slot = slot;
    let _ = sock.set_nodelay(true);
    let kill = Kill::default();
    let stream = ConnStream::new(sock, sh.read_rate, sh.read_burst, kill.clone());
    let started = Arc::new(AtomicBool::new(false));
    let handler = Conn { sh: sh.clone(), key, user: String::new(), admin: false, opened: false, started: started.clone() };
    // The SSH version exchange happens inside run_stream: bound it too.
    let running = match tokio::time::timeout(HANDSHAKE_TIMEOUT, russh::server::run_stream(sh.config.clone(), stream, handler)).await {
        Ok(Ok(r)) => r,
        _ => return,
    };
    tokio::pin!(running);
    tokio::select! {
        _ = &mut running => return,
        _ = tokio::time::sleep(HANDSHAKE_TIMEOUT) => {}
    }
    if !started.load(Ordering::Relaxed) {
        kill.kill(); // never opened a chat session
    }
    let _ = running.await;
}

/// One SSH connection.
struct Conn {
    sh: Arc<Shared>,
    key: String,
    user: String,
    admin: bool,
    opened: bool,
    started: Arc<AtomicBool>,
}

impl Conn {
    fn is_admin_user(&self, user: &str) -> bool {
        !self.sh.admins.is_empty() && user.to_lowercase() == self.sh.admin_user
    }
    fn is_admin_key(&self, key: &PublicKey) -> bool {
        self.sh.admins.iter().any(|a| a.key_data() == key.key_data())
    }
}

impl russh::server::Handler for Conn {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        if self.is_admin_user(user) {
            // admin login requires a key
            return Ok(Auth::Reject { proceed_with_methods: Some(MethodSet::from(&[MethodKind::PublicKey][..])), partial_success: false });
        }
        self.user = user.to_string();
        Ok(Auth::Accept)
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        _response: Option<russh::server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if self.is_admin_user(user) {
            return Ok(Auth::reject());
        }
        self.user = user.to_string();
        Ok(Auth::Accept)
    }

    async fn auth_publickey_offered(&mut self, user: &str, offer: russh::server::PublicKeyOffer<'_>) -> Result<Auth, Self::Error> {
        if self.is_admin_user(user) && self.is_admin_key(offer.public_key) { Ok(Auth::Accept) } else { Ok(Auth::reject()) }
    }

    async fn auth_publickey(&mut self, user: &str, key: russh::server::VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        if self.is_admin_user(user) && self.is_admin_key(key.public_key) {
            self.user = user.to_string();
            self.admin = true;
            return Ok(Auth::Accept);
        }
        Ok(Auth::reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.opened {
            // one chat session per connection
            reply.reject(russh::ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        }
        self.opened = true;
        self.started.store(true, Ordering::Relaxed);
        reply.accept().await;
        let s = SessionTask {
            sh: self.sh.clone(),
            key: self.key.clone(),
            user: self.user.clone(),
            admin: self.admin,
            handle: session.handle(),
        };
        tokio::spawn(s.run(channel));
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _w: u32,
        _h: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn exec_request(&mut self, channel: ChannelId, _data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel) // the session task explains and closes
    }

    async fn subsystem_request(&mut self, channel: ChannelId, _name: &str, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel) // likewise (sftp, scp)
    }

    async fn env_request(&mut self, channel: ChannelId, _k: &str, _v: &str, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_failure(channel)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _: bool,
        _: &str,
        _: &str,
        _: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)
    }
}

/// `ssh -T bot@host` (or json@ / agent@): JSON Lines mode for AI agents.
fn is_agent_user(u: &str) -> bool {
    matches!(u.to_lowercase().as_str(), "bot" | "json" | "agent")
}

const ONLY_CHAT: &str =
    "这里只有聊天室，请不带命令连接 · This server only offers the chat room. Connect without a command:  ssh -p 2222 guest@<host>";

struct SessionTask {
    sh: Arc<Shared>,
    key: String,
    user: String,
    admin: bool,
    handle: russh::server::Handle,
}

struct Pty {
    term: String,
    w: usize,
    h: usize,
}

impl SessionTask {
    async fn run(self, channel: Channel<Msg>) {
        let id = channel.id();
        let (mut read, write) = channel.split();
        // Wait for the client to say what it wants.
        let mut pty: Option<Pty> = None;
        loop {
            match read.wait().await {
                Some(ChannelMsg::RequestPty { term, col_width, row_height, .. }) => {
                    // clients without a real terminal (ssh -tt from a script) report 0x0
                    let (w, h) = if col_width == 0 || row_height == 0 { (80, 24) } else { (col_width as usize, row_height as usize) };
                    pty = Some(Pty { term, w, h })
                }
                Some(ChannelMsg::WindowChange { col_width, row_height, .. }) => {
                    if let Some(p) = pty.as_mut() {
                        p.w = col_width as usize;
                        p.h = row_height as usize;
                    }
                }
                Some(ChannelMsg::RequestShell { .. }) => break,
                Some(ChannelMsg::Exec { .. }) | Some(ChannelMsg::RequestSubsystem { .. }) => {
                    let _ = write.extended_data_bytes(1, format!("{ONLY_CHAT}\n").into_bytes()).await;
                    let _ = write.exit_status(1).await;
                    let _ = write.eof().await;
                    let _ = write.close().await;
                    return;
                }
                Some(_) => {}
                None => return,
            }
        }
        let nl = if pty.is_some() { "\r\n" } else { "\n" };
        let hub = self.sh.hub.clone();
        if self.admin {
            log_info!("admin session started");
        }
        let mut client = match hub.connect(&self.key, self.admin) {
            Ok(c) => c,
            Err(e) => {
                let msg = refusal(e);
                let _ = write.extended_data_bytes(1, format!("{msg}{nl}").into_bytes()).await;
                let _ = write.exit_status(1).await;
                let _ = write.eof().await;
                let _ = write.close().await;
                return;
            }
        };
        // Input from the client, as UI events; ends with Eof.
        let (in_tx, in_rx) = mpsc::channel::<Input>(64);
        tokio::spawn(async move {
            loop {
                let inp = match read.wait().await {
                    Some(ChannelMsg::Data { data }) => Input::Data(data.to_vec()),
                    Some(ChannelMsg::WindowChange { col_width, row_height, .. }) => Input::Resize(col_width as usize, row_height as usize),
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        let _ = in_tx.send(Input::Eof).await;
                        return;
                    }
                    Some(_) => continue,
                };
                if in_tx.send(inp).await.is_err() {
                    return;
                }
            }
        });
        let mut io = Io { input: in_rx, out: Box::pin(write.make_writer()) };
        let user = self.user.to_lowercase();
        if is_agent_user(&user) {
            hub.mark_bot(client.id);
            ui::agent::run(&mut io, &hub, &mut client).await;
        } else {
            let plain = match &pty {
                None => true,
                Some(p) => user == "plain" || user == "text" || p.term.is_empty() || p.term.eq_ignore_ascii_case("dumb"),
            };
            if plain {
                let w = pty.as_ref().map_or(80, |p| p.w);
                ui::plain::run(&mut io, &hub, &mut client, pty.is_some(), w).await;
            } else {
                // `ssh white@host`, `ssh green@host`, `ssh black@host` pick the
                // background at login; any other user name keeps the default.
                hub.set_theme(client.id, &user);
                let p = pty.as_ref().unwrap();
                let msg = ui::term::run(&mut io, &hub, &mut client, p.w, p.h).await;
                if !msg.is_empty() {
                    io.write(format!("{msg}\r\n").as_bytes()).await;
                }
            }
        }
        hub.disconnect(client.id);
        drop(io);
        let _ = write.exit_status(0).await;
        let _ = write.eof().await;
        let _ = write.close().await;
        let _ = self.handle.close(id).await;
    }
}

fn refusal(e: ConnectError) -> String {
    format!("抱歉：{} · Sorry: {}", e.zh(), e.en())
}

fn load_authorized_keys(path: &Path) -> Result<Vec<PublicKey>, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut keys = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        keys.push(parse_authorized_key(line).map_err(|e| format!("parse {line:?}: {e}"))?);
    }
    Ok(keys)
}

/// One authorized_keys line: `[options] keytype base64 [comment]`. Options
/// (`from="…"`, `no-pty`, …) are skipped — this file only says who is admin.
fn parse_authorized_key(line: &str) -> Result<PublicKey, String> {
    if let Ok(e) = line.parse::<ssh_key::authorized_keys::Entry>() {
        return Ok(e.public_key().clone());
    }
    let types = ["ssh-ed25519", "ssh-rsa", "ecdsa-sha2-", "sk-ssh-ed25519@openssh.com", "sk-ecdsa-sha2-"];
    let mut rest = line;
    loop {
        let t = rest.trim_start();
        if types.iter().any(|p| t.starts_with(p)) {
            return PublicKey::from_openssh(t).map_err(|e| e.to_string());
        }
        // skip one option, honouring quotes: from="a b,c",no-pty
        let mut in_q = false;
        let mut cut = None;
        for (i, c) in t.char_indices() {
            match c {
                '"' => in_q = !in_q,
                ' ' | '\t' if !in_q => {
                    cut = Some(i);
                    break;
                }
                _ => {}
            }
        }
        match cut {
            Some(i) => rest = &t[i..],
            None => return Err("no key found".into()),
        }
    }
}

/// Loads the host key (OpenSSH format, as written by the Go version and
/// ssh-keygen), creating an Ed25519 key if there is none yet.
fn load_or_create_host_key(path: &Path) -> Result<ssh_key::PrivateKey, String> {
    match std::fs::read_to_string(path) {
        Ok(pem) => return russh::keys::decode_secret_key(&pem, None).map_err(|e| e.to_string()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        Err(_) => {}
    }
    let mut seed = [0u8; 32];
    crate::chat::random_bytes(&mut seed);
    let key = ssh_key::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&seed));
    let pem = key.to_openssh(LineEnding::LF).map_err(|e| e.to_string())?;
    bans::write_file_atomic(path, pem.as_bytes()).map_err(|e| e.to_string())?;
    if let Ok(public) = key.public_key().to_openssh() {
        let mut p = path.as_os_str().to_owned();
        p.push(".pub");
        let _ = bans::write_file_atomic(Path::new(&p), format!("{public}\n").as_bytes());
    }
    log_info!("generated host key");
    Ok(key)
}

#[cfg(test)]
mod tests;
