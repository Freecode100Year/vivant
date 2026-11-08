//! VIVANT: an anonymous chat room that anyone can join with nothing but an
//! SSH client:
//!
//! ```text
//! ssh -p 2222 guest@term.example.com
//! ```

mod chat;
mod flags;
mod log;
mod server;
mod ui;

use chat::{Config, Hub, RoomDef, sanitize::sanitize, sanitize::valid_room_name};
use flags::FlagSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

// musl's allocator is slow under many threads; mimalloc is the same
// everywhere and keeps memory per session low.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Set at build time: `VIVANT_VERSION=v1.2.3 cargo build --release`.
pub const VERSION: &str = match option_env!("VIVANT_VERSION") {
    Some(v) => v,
    None => "dev",
};

/// VIVANT_<k>, falling back to the pre-rename PTT_<k>.
fn env_or(k: &str, def: &str) -> String {
    for p in ["VIVANT_", "PTT_"] {
        if let Ok(v) = std::env::var(format!("{p}{k}"))
            && !v.is_empty()
        {
            return v;
        }
    }
    def.to_string()
}

fn main() {
    let mut fs = FlagSet::new();
    fs.string("addr", &env_or("ADDR", ":2222"), "监听地址 · listen address");
    fs.string(
        "host-key",
        &env_or("HOST_KEY", "./data/host_ed25519"),
        "SSH 主机密钥路径，不存在会自动生成 · SSH host key path (created if missing)",
    );
    fs.string(
        "admin-keys",
        &env_or("ADMIN_KEYS", ""),
        "管理员公钥文件（authorized_keys 格式，可选）· authorized_keys file whose keys get admin rights (optional)",
    );
    fs.string(
        "rooms",
        &env_or("ROOMS", ""),
        "公共房间，逗号分隔 name[:主题]，第一个是大厅 · public rooms, comma separated name[:topic] (first = lobby)",
    );
    fs.int("history", chat::DEFAULT_HISTORY_SIZE as i64, "每个房间保留的消息条数 · messages kept per room");
    fs.duration(
        "history-ttl",
        chat::DEFAULT_HISTORY_TTL,
        "消息保留时长（只在内存，重启清空）· discard messages older than this (memory only, cleared on restart)",
    );
    fs.int("max-clients", 1000, "最大同时在线会话数 · maximum concurrent sessions");
    fs.int("max-per-ip", 3, "每个地址最多会话数（IPv6 按 /64）· maximum sessions per address (IPv6 per /64)");
    fs.int("conn-per-min", 10, "每个地址每分钟最多新会话 · new sessions per address per minute");
    fs.int(
        "read-rate",
        32 << 10,
        "每个连接每秒最多接收字节数，超出会被放慢（0 = 不限）· bytes/s one connection may send before it is slowed down (0 = off)",
    );
    fs.int("max-tcp-per-ip", 8, "每个地址最多 TCP 连接数（含未认证的）· raw TCP connections per address, including unauthenticated ones");
    fs.string(
        "admin-user",
        &env_or("ADMIN_USER", "admin"),
        "管理员登录用户名（会校验公钥），其他人从不发送公钥 · username admins log in with (key checked); everyone else never sends a key",
    );
    fs.string(
        "alt-addr",
        &env_or("ALT_ADDR", ""),
        "额外监听地址，逗号分隔，例如 :443 · extra listen addresses, comma separated, e.g. :443",
    );
    fs.bool(
        "tor-exits",
        env_or("TOR_EXITS", "on") != "off",
        "下载 Tor 出口节点列表，让 Tor 用户共用宽松限额 · fetch Tor's exit list so Tor users share the generous Tor limits",
    );
    fs.int("quiet-above", 20, "房间超过这个人数后，进出提示每 30 秒汇总一次（-1 = 不汇总）· above this many members, join/leave lines are summarised every 30s (-1 = never)");
    fs.string(
        "ban-file",
        &env_or("BAN_FILE", ""),
        "把封禁保存到这个文件，重启后保留（可选，有隐私代价，见文档）· persist bans in this file (opt-in; privacy trade-off, see docs)",
    );
    fs.string(
        "onion-addr",
        &env_or("ONION_ADDR", ""),
        "给 Tor 洋葱服务用的本地监听地址，例如 127.0.0.1:2224 · extra listener for a Tor onion service, e.g. 127.0.0.1:2224",
    );
    fs.float("msg-rate", 1.0, "每秒允许发送的消息数（持续）· messages per second allowed (sustained)");
    fs.float("msg-burst", 5.0, "允许连续发送的消息数 · message burst size");
    fs.int("max-len", 500, "单条消息最大字数 · maximum message length in characters");
    fs.duration(
        "idle-timeout",
        Duration::from_secs(3600),
        "多久不发言就断开（0 = 不断开）· disconnect sessions silent for this long (0 = never)",
    );
    fs.string(
        "motd",
        &env_or("MOTD", ""),
        "进入时额外显示的公告文本文件（可选）· file with extra welcome lines shown on connect (optional)",
    );
    fs.string(
        "health-addr",
        &env_or("HEALTH_ADDR", ""),
        "在此地址提供 GET /healthz（只返回计数），例如 127.0.0.1:2223 · serve GET /healthz (counts only) here, e.g. 127.0.0.1:2223",
    );
    fs.bool("v", false, "调试日志 · debug logging");
    fs.bool("version", false, "显示版本号后退出 · print version and exit");

    let usage = "VIVANT — 匿名 SSH 聊天室 · anonymous SSH chat room\n\n用法 · Usage: vivant [选项 · options]\n环境变量 · Environment: VIVANT_ADDR, VIVANT_ROOMS, … (旧名 · legacy PTT_*)\n\n";
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = fs.parse(&args) {
        if e == flags::HELP {
            eprint!("{usage}{}", fs.defaults());
            std::process::exit(0);
        }
        eprintln!("{e}");
        eprint!("{usage}{}", fs.defaults());
        std::process::exit(2);
    }
    if fs.get_bool("version") {
        println!("vivant {VERSION}");
        return;
    }
    log::set_debug(fs.get_bool("v"));

    let rooms = match parse_rooms(&fs.get_string("rooms")) {
        Ok(r) => r,
        Err(e) => {
            log_error!("bad -rooms", err = e);
            std::process::exit(2);
        }
    };
    let mut motd = Vec::new();
    let motd_path = fs.get_string("motd");
    if !motd_path.is_empty() {
        match std::fs::read_to_string(&motd_path) {
            Ok(data) => motd = data.trim_end_matches('\n').split('\n').map(|l| sanitize(l, 200)).collect(),
            Err(e) => {
                log_error!("cannot read -motd", err = e);
                std::process::exit(2);
            }
        }
    }
    let idle = fs.get_duration("idle-timeout");
    let tor_exits = fs.get_bool("tor-exits");
    let ban_file = fs.get_string("ban-file");
    let onion_addr = fs.get_string("onion-addr");
    let max_clients = fs.get_int("max-clients").max(0) as usize;
    let read_rate = fs.get_int("read-rate").max(0) as usize;
    let hub = Hub::new(Config {
        public_rooms: rooms,
        history_size: fs.get_int("history").max(0) as usize,
        history_ttl: fs.get_duration("history-ttl"),
        max_msg_runes: fs.get_int("max-len").max(0) as usize,
        max_clients,
        max_conns_per_ip: fs.get_int("max-per-ip").max(0) as usize,
        conn_per_minute: fs.get_int("conn-per-min").max(0) as usize,
        msg_per_second: fs.get_float("msg-rate"),
        msg_burst: fs.get_float("msg-burst"),
        idle_kick: idle,
        motd,
        persist_bans: !ban_file.is_empty(),
        tor_enabled: !onion_addr.is_empty(),
        tor_exits,
        quiet_above: fs.get_int("quiet-above"),
        ..Default::default()
    });
    let addr = fs.get_string("addr");
    let opt = server::Options {
        addr: addr.clone(),
        host_key_path: fs.get_string("host-key").into(),
        admin_keys_path: Some(fs.get_string("admin-keys")).filter(|s| !s.is_empty()).map(Into::into),
        // the transport-level limit only catches silent connections; the
        // chat-level one (what users send) is the hub's, so this is longer
        idle_timeout: if idle.is_zero() { Duration::ZERO } else { idle + Duration::from_secs(300) },
        max_tcp_per_ip: fs.get_int("max-tcp-per-ip").max(0) as usize,
        max_tcp: 2 * hub.config().max_clients + 500,
        read_rate,
        read_burst: 8 * read_rate,
        admin_user: fs.get_string("admin-user"),
        onion_addr,
        onion_max_tcp: 0,
        ban_file: Some(ban_file).filter(|s| !s.is_empty()).map(Into::into),
        alt_addrs: fs.get_string("alt-addr").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        tor_exits,
    };
    let health_addr = fs.get_string("health-addr");

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let code = rt.block_on(async move {
        let srv = match server::Server::new(hub.clone(), opt) {
            Ok(s) => s,
            Err(e) => {
                log_error!("could not create server", err = e);
                return 1;
            }
        };
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(async move {
            wait_for_signal().await;
            let _ = stop_tx.send(true);
        });
        if !health_addr.is_empty() {
            tokio::spawn(serve_health(health_addr, hub.clone(), stop_rx.clone()));
        }
        log_info!("VIVANT listening", addr = addr, version = VERSION);
        match srv.run(stop_rx).await {
            Ok(()) => {
                log_info!("stopped");
                0
            }
            Err(e) => {
                log_error!("server error", err = e);
                1
            }
        }
    });
    rt.shutdown_timeout(Duration::from_secs(1));
    std::process::exit(code);
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn parse_rooms(s: &str) -> Result<Vec<RoomDef>, String> {
    if s.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<RoomDef> = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        let (name, topic) = part.split_once(':').unwrap_or((part, ""));
        let name = name.trim().trim_start_matches('#').to_lowercase();
        if !valid_room_name(&name) || name.starts_with("p-") {
            return Err(format!("invalid room name {name:?}"));
        }
        if out.iter().any(|r| r.name == name) {
            return Err(format!("duplicate room {name:?}"));
        }
        out.push(RoomDef { name, topic: topic.trim().to_string(), topic_en: String::new() });
    }
    Ok(out)
}

/// GET /healthz: counts only — no nicknames, addresses or messages.
async fn serve_health(addr: String, hub: Arc<Hub>, mut stop: watch::Receiver<bool>) {
    let l = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            log_error!("health endpoint failed", err = e);
            return;
        }
    };
    log_info!("health endpoint", addr = addr);
    loop {
        let sock = tokio::select! {
            r = l.accept() => match r {
                Ok((s, _)) => s,
                Err(_) => continue,
            },
            _ = stop.wait_for(|s| *s) => return,
        };
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut s = sock;
            let mut buf = vec![0u8; 4096];
            let mut n = 0;
            let read = tokio::time::timeout(Duration::from_secs(5), async {
                while n < buf.len() {
                    match s.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(k) => {
                            n += k;
                            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
            })
            .await;
            if read.is_err() {
                return;
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let (status, body) = if req.starts_with("GET /healthz ") || req.starts_with("GET /healthz?") {
                let st = hub.stats();
                let body = serde_json::json!({
                    "status": "ok",
                    "version": VERSION,
                    "online": st.online,
                    "rooms": st.rooms,
                    "private_rooms": st.private_rooms,
                    "uptime_seconds": st.uptime_seconds,
                });
                ("200 OK", format!("{body}\n"))
            } else {
                ("404 Not Found", "404 page not found\n".to_string())
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
        });
    }
}
