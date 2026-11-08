//! In-memory chat state: rooms, anonymous identities, private messages,
//! short-lived history and abuse protection. It knows nothing about SSH or
//! terminals; front-ends talk to it through `connect`, `handle`, `status` and
//! the client's event channel.
//!
//! Concurrency: all state sits behind one `std::sync::Mutex`. Every method
//! takes the lock, does a bounded amount of work and returns — never holding
//! it across an `.await`. Delivery to clients is a non-blocking `try_send`
//! into a bounded per-client queue: a client that stops reading loses events
//! instead of stalling the room.

pub mod sanitize;
pub mod zone;

use chrono::{DateTime, Utc};
use sanitize::{is_punct, is_space, reserved_nick, sanitize, valid_nick};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
pub use zone::{Zone, parse_zone};

/// Classifies an event so front-ends can style it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Kind {
    #[default]
    Chat, // <nick> text
    Action,  // * nick does something
    System,  // joins, leaves, nick changes
    DmIn,    // private message received
    DmOut,   // private message sent (echo to sender)
    Info,    // command output for this user only
    Error,   // command error for this user only
    Notice,  // admin broadcast
    Mention, // someone @mentioned you in a room you're not in
    History, // a room's history replayed on join, as one event (see batch)
}

/// One line (or block) of output for a client. Events are shared between
/// recipients behind an `Arc`: treat them as read-only.
#[derive(Clone, Debug, Default)]
pub struct Event {
    /// non-zero for events broadcast to a room (same for every viewer)
    pub id: u64,
    pub time: DateTime<Utc>,
    pub kind: Kind,
    pub room: String,
    pub from: String,
    pub to: String,
    pub text: String,
    /// extra lines for multi-line info blocks
    pub lines: Vec<String>,
    /// English half of a server message (`text`/`lines` hold the Chinese).
    /// Empty `lines_en` means `lines` are already bilingual. User messages
    /// have no English half.
    pub en: String,
    pub lines_en: Vec<String>,
    /// replayed from the room buffer on join
    pub history: bool,
    /// the replayed messages of a History event, oldest first
    pub batch: Vec<Arc<Event>>,
    /// client id of the author (never shown), for /forget
    pub(crate) sender: u64,
}

impl Event {
    pub fn new(kind: Kind, text: impl Into<String>, en: impl Into<String>) -> Event {
        Event { time: Utc::now(), kind, text: text.into(), en: en.into(), ..Default::default() }
    }
}

/// A public room created at startup.
#[derive(Clone, Debug, Default)]
pub struct RoomDef {
    pub name: String,
    pub topic: String,
    pub topic_en: String,
}

/// Tunes the hub. Zero values are replaced with defaults by `Hub::new`.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub public_rooms: Vec<RoomDef>,
    pub history_size: usize,
    pub history_ttl: Duration,
    pub private_history_size: usize,
    pub max_msg_runes: usize,
    pub max_clients: usize,
    /// concurrent sessions per address (IPv6: per /64)
    pub max_conns_per_ip: usize,
    /// new sessions per address per minute
    pub conn_per_minute: usize,
    pub msg_burst: f64,
    pub msg_per_second: f64,
    pub nick_cooldown: Duration,
    pub max_private: usize,
    /// drop sessions that haven't sent anything for this long (zero = never)
    pub idle_kick: Duration,
    /// the same for bot-mode sessions (`ssh -T bot@host`), which usually stay
    /// connected for good (zero = never)
    pub bot_idle_kick: Duration,
    /// a chosen nickname stays reserved for its address this long after the
    /// session ends, so a reconnecting person or agent gets it back
    pub nick_hold: Duration,
    pub onion_max_conns: usize,
    pub onion_per_min: usize,
    pub motd: Vec<String>,
    // Facts about the deployment that /privacy reports.
    pub persist_bans: bool,
    pub tor_enabled: bool,
    pub tor_exits: bool,
    /// In rooms with more members than this, join/leave lines are replaced by
    /// a periodic summary. 0 = default (20); negative = never collapse.
    pub quiet_above: i64,
}

pub const DEFAULT_HISTORY_SIZE: usize = 500;
pub const DEFAULT_HISTORY_TTL: Duration = Duration::from_secs(24 * 3600);

/// The address key shared by every session that arrives through Tor.
pub const ONION_KEY: &str = "onion";

impl Config {
    fn defaults(&mut self) {
        if self.public_rooms.is_empty() {
            let d = |n: &str, t: &str, e: &str| RoomDef { name: n.into(), topic: t.into(), topic_en: e.into() };
            self.public_rooms = vec![
                d("general", "大厅 · 什么都可以聊", "Lobby · anything goes"),
                d("chinese", "中文频道", "Chinese channel"),
                d("english", "英文频道", "English channel"),
                d("technology", "编程、Linux、网络、安全", "Programming, Linux, networks, security"),
                d("anonymous", "随便聊", "Off-topic"),
            ];
        }
        let dz = |v: &mut usize, d: usize| {
            if *v == 0 {
                *v = d
            }
        };
        dz(&mut self.history_size, DEFAULT_HISTORY_SIZE);
        if self.history_ttl.is_zero() {
            self.history_ttl = DEFAULT_HISTORY_TTL;
        }
        dz(&mut self.private_history_size, self.history_size.min(100));
        dz(&mut self.max_msg_runes, 500);
        dz(&mut self.max_clients, 1000);
        dz(&mut self.max_conns_per_ip, 3);
        dz(&mut self.conn_per_minute, 10);
        if self.msg_burst <= 0.0 {
            self.msg_burst = 5.0;
        }
        if self.msg_per_second <= 0.0 {
            self.msg_per_second = 1.0;
        }
        if self.nick_hold.is_zero() {
            self.nick_hold = Duration::from_secs(600);
        }
        if self.nick_cooldown.is_zero() {
            self.nick_cooldown = Duration::from_secs(20);
        }
        dz(&mut self.max_private, 500);
        if self.quiet_above == 0 {
            self.quiet_above = 20;
        }
        dz(&mut self.onion_max_conns, 300);
        dz(&mut self.onion_per_min, 120);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    Banned,
    Full,
    TooManyConns,
    ConnRate,
}

impl ConnectError {
    pub fn en(&self) -> &'static str {
        match self {
            ConnectError::Banned => "this address is temporarily banned",
            ConnectError::Full => "server is full, try again later",
            ConnectError::TooManyConns => "too many sessions from your address",
            ConnectError::ConnRate => "connecting too fast, wait a minute",
        }
    }
    pub fn zh(&self) -> &'static str {
        match self {
            ConnectError::Banned => "这个地址暂时被封禁了",
            ConnectError::Full => "服务器已满，请稍后再试",
            ConnectError::TooManyConns => "你的地址连接数太多了",
            ConnectError::ConnRate => "连接太频繁，请等一分钟",
        }
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.en())
    }
}

/// Events buffered per client before new ones are dropped.
const CLIENT_QUEUE: usize = 128;

/// The front-end's side of a connected session.
pub struct Client {
    pub id: u64,
    pub rx: mpsc::Receiver<Arc<Event>>,
    /// becomes `Some(reason)` when the hub drops the client (kick, ban, idle,
    /// shutdown, or a normal disconnect with an empty reason)
    pub done: watch::Receiver<Option<String>>,
}

struct Room {
    name: String,
    topic: String,
    topic_en: String,
    private: bool,
    members: HashSet<u64>,
    history: VecDeque<Arc<Event>>,
    /// chat/action messages ever posted, for unread counts
    seq: u64,
    joins: usize,
    leaves: usize,
}

impl Room {
    fn new(name: &str, topic: &str, topic_en: &str, private: bool) -> Room {
        Room {
            name: name.into(),
            topic: topic.into(),
            topic_en: topic_en.into(),
            private,
            members: HashSet::new(),
            history: VecDeque::new(),
            seq: 0,
            joins: 0,
            leaves: 0,
        }
    }

    fn topic_bi(&self) -> String {
        if self.topic_en.is_empty() || self.topic_en == self.topic { self.topic.clone() } else { bi(&self.topic, &self.topic_en) }
    }
}

struct ClientState {
    tx: mpsc::Sender<Arc<Event>>,
    done: watch::Sender<Option<String>>,
    ip_key: String,
    admin: bool,
    nick: String,
    room: String,
    last_dm_from: String,
    tokens: f64,
    tokens_at: Instant,
    last_text: String,
    repeat: u32,
    violations: u32,
    violation_at: Option<Instant>,
    muted_until: Option<Instant>,
    nick_at: Option<Instant>,
    last_active: Instant,
    ignores: HashSet<u64>,
    loc: Zone,
    /// room -> room.seq when the client last left/joined it
    seen: HashMap<String, u64>,
    /// rooms where the client was @mentioned while away
    mentioned: HashSet<String>,
    no_bell: bool,
    /// a bot-mode session (only changes the idle limit; never shown)
    bot: bool,
    theme: &'static str,
}

impl ClientState {
    fn send(&self, ev: Arc<Event>) {
        let _ = self.tx.try_send(ev);
    }
    fn send_ev(&self, ev: Event) {
        self.send(Arc::new(ev));
    }
    fn close(&self, reason: &str) {
        self.done.send_if_modified(|v| {
            if v.is_none() {
                *v = Some(reason.to_string());
                true
            } else {
                false
            }
        });
    }
}

/// UI-level effects of a command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub quit: bool,
    pub clear: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    pub name: String,
    pub label: String,
    pub users: usize,
    pub current: bool,
    pub private: bool,
    pub unread: u64,
    pub mentioned: bool,
}

/// The current view of the world for one client.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub nick: String,
    pub room: String,
    pub room_label: String,
    pub topic: String,
    pub online: usize,
    pub room_users: usize,
    pub rooms: Vec<RoomInfo>,
    pub uptime: Duration,
    pub admin: bool,
    pub muted: bool,
    pub bell: bool,
    pub loc: Zone,
    pub theme: &'static str,
}

/// A privacy-safe summary for health checks: counts only.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Stats {
    pub online: usize,
    pub rooms: usize,
    pub private_rooms: usize,
    pub uptime_seconds: u64,
}

pub const THEME_DARK: &str = "dark";
pub const THEME_LIGHT: &str = "light";
pub const THEME_GREEN: &str = "green";
const THEME_ORDER: [&str; 3] = [THEME_GREEN, THEME_DARK, THEME_LIGHT];

pub fn parse_theme(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().as_str() {
        "dark" | "black" | "黑" | "黑色" | "暗" | "深色" => Some(THEME_DARK),
        "light" | "white" | "白" | "白色" | "亮" | "浅色" => Some(THEME_LIGHT),
        "green" | "eye" | "eyecare" | "eye-care" | "绿" | "绿色" | "护眼" | "护眼绿" => Some(THEME_GREEN),
        _ => None,
    }
}

fn theme_label(t: &str) -> (&'static str, &'static str) {
    match t {
        THEME_LIGHT => ("白色", "white"),
        THEME_GREEN => ("护眼绿", "eye-care green"),
        _ => ("黑色", "black"),
    }
}

pub fn bi(zh: &str, en: &str) -> String {
    format!("{zh} · {en}")
}

/// Formats a duration as "24 小时"/"24h", "10 分钟"/"10m".
pub fn dur_text(d: Duration) -> (String, String) {
    let s = d.as_secs();
    if s >= 3600 && s % 3600 == 0 && d.subsec_nanos() == 0 {
        (format!("{} 小时", s / 3600), format!("{}h", s / 3600))
    } else if s >= 60 && s % 60 == 0 && d.subsec_nanos() == 0 {
        (format!("{} 分钟", s / 60), format!("{}m", s / 60))
    } else {
        let t = go_duration(d);
        (t.clone(), t)
    }
}

pub use crate::flags::go_duration;

struct State {
    next_id: u64,
    clients: HashMap<u64, ClientState>,
    by_nick: HashMap<String, u64>,
    rooms: HashMap<String, Room>,
    public: Vec<String>,
    bans: HashMap<String, DateTime<Utc>>,
    per_ip: HashMap<String, usize>,
    conn_log: HashMap<String, VecDeque<Instant>>,
    reports: VecDeque<String>,
    event_id: u64,
    ban_ver: u64,
    shutdown: bool,
    /// lowercased nick -> (address key, until): nicknames held for their
    /// previous owner after a disconnect
    held: HashMap<String, (String, Instant)>,
}

/// The whole chat server state.
pub struct Hub {
    cfg: Config,
    started: Instant,
    st: Mutex<State>,
}

/// Sends a Chinese + English info line to a client.
macro_rules! info {
    ($c:expr, $zh:expr, $en:expr) => {
        $c.send_ev(Event::new(Kind::Info, $zh, $en))
    };
}
macro_rules! err {
    ($c:expr, $zh:expr, $en:expr) => {
        $c.send_ev(Event::new(Kind::Error, $zh, $en))
    };
}

fn q(s: &str) -> String {
    format!("{s:?}")
}

impl Hub {
    pub fn new(mut cfg: Config) -> Arc<Hub> {
        cfg.defaults();
        let mut rooms = HashMap::new();
        let mut public = Vec::new();
        for r in &cfg.public_rooms {
            rooms.insert(r.name.clone(), Room::new(&r.name, &r.topic, &r.topic_en, false));
            public.push(r.name.clone());
        }
        Arc::new(Hub {
            started: Instant::now(),
            st: Mutex::new(State {
                next_id: 0,
                clients: HashMap::new(),
                by_nick: HashMap::new(),
                rooms,
                public,
                bans: HashMap::new(),
                per_ip: HashMap::new(),
                conn_log: HashMap::new(),
                reports: VecDeque::new(),
                event_id: 0,
                ban_ver: 0,
                shutdown: false,
                held: HashMap::new(),
            }),
            cfg,
        })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock would be a bug, but it must not take
        // every other session down with it.
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers a new anonymous session coming from `ip_key` (an opaque,
    /// already-hashed address key). The client starts in the first public
    /// room.
    pub fn connect(&self, ip_key: &str, admin: bool) -> Result<Client, ConnectError> {
        let now = Instant::now();
        let wall = Utc::now();
        let mut st = self.lock();
        let st = &mut *st;
        if st.shutdown {
            return Err(ConnectError::Full);
        }
        if let Some(until) = st.bans.get(ip_key).copied() {
            if wall < until && !admin {
                return Err(ConnectError::Banned);
            }
            if wall >= until {
                st.bans.remove(ip_key);
                st.ban_ver += 1;
            }
        }
        if !admin {
            let (max_conns, per_min) = if ip_key == ONION_KEY {
                (self.cfg.onion_max_conns, self.cfg.onion_per_min)
            } else {
                (self.cfg.max_conns_per_ip, self.cfg.conn_per_minute)
            };
            if st.clients.len() >= self.cfg.max_clients {
                return Err(ConnectError::Full);
            }
            if st.per_ip.get(ip_key).copied().unwrap_or(0) >= max_conns {
                return Err(ConnectError::TooManyConns);
            }
            let log = st.conn_log.entry(ip_key.to_string()).or_default();
            prune_times(log, now - Duration::from_secs(60));
            if log.len() >= per_min {
                return Err(ConnectError::ConnRate);
            }
            log.push_back(now);
        }

        st.next_id += 1;
        let id = st.next_id;
        let (tx, rx) = mpsc::channel(CLIENT_QUEUE);
        let (done_tx, done_rx) = watch::channel(None);
        let nick = free_nick(&st.by_nick);
        let mut seen = HashMap::new();
        for name in &st.public {
            seen.insert(name.clone(), st.rooms[name].seq); // unread counts start from "now"
        }
        let c = ClientState {
            tx,
            done: done_tx,
            ip_key: ip_key.to_string(),
            admin,
            nick: nick.clone(),
            room: String::new(),
            last_dm_from: String::new(),
            tokens: self.cfg.msg_burst,
            tokens_at: now,
            last_text: String::new(),
            repeat: 0,
            violations: 0,
            violation_at: None,
            muted_until: None,
            nick_at: None,
            bot: false,
            last_active: now,
            ignores: HashSet::new(),
            loc: Zone::Local,
            seen,
            mentioned: HashSet::new(),
            no_bell: false,
            theme: THEME_GREEN,
        };
        let mut welcome = Event::new(
            Kind::Info,
            format!("欢迎，{nick}！直接打字就能聊天，/help 查看命令。"),
            format!("Welcome, {nick}! Just type to chat; /help lists commands."),
        );
        welcome.lines = vec!["服务器能看到聊天内容，别发能认出你的信息 · /privacy".into()];
        welcome.lines_en = vec!["The server can read messages — don't post anything that identifies you · /privacy".into()];
        c.send_ev(welcome);
        if let Some((first, rest)) = self.cfg.motd.split_first() {
            let mut ev = Event::new(Kind::Notice, first.clone(), "");
            ev.lines = rest.to_vec();
            c.send_ev(ev);
        }
        st.by_nick.insert(nick.to_lowercase(), id);
        *st.per_ip.entry(ip_key.to_string()).or_insert(0) += 1;
        st.clients.insert(id, c);
        let lobby = st.public[0].clone();
        self.join(st, id, &lobby);
        Ok(Client { id, rx, done: done_rx })
    }

    /// Removes a client (idempotent).
    pub fn disconnect(&self, id: u64) {
        let mut st = self.lock();
        self.remove(&mut st, id, "", "left");
    }

    fn remove(&self, st: &mut State, id: u64, reason: &str, verb: &str) {
        let Some(c) = st.clients.remove(&id) else { return };
        let room = c.room.clone();
        if let Some(r) = st.rooms.get_mut(&room) {
            r.members.remove(&id);
        }
        self.presence(st, &room, &c.nick, verb, verb == "left" || verb == "timed out");
        self.gc_room(st, &room);
        st.by_nick.remove(&c.nick.to_lowercase());
        if c.nick_at.is_some() {
            // a chosen name (not a random anonymous_N): keep it for whoever
            // reconnects from the same address
            st.held.insert(c.nick.to_lowercase(), (c.ip_key.clone(), Instant::now() + self.cfg.nick_hold));
        }
        match st.per_ip.get_mut(&c.ip_key) {
            Some(n) if *n > 1 => *n -= 1,
            _ => {
                st.per_ip.remove(&c.ip_key);
            }
        }
        c.close(reason);
    }

    /// Tells every client the server is going away and drops them.
    pub fn shutdown(&self, msg: &str) {
        let mut st = self.lock();
        st.shutdown = true;
        for c in st.clients.values() {
            c.send_ev(Event::new(Kind::Notice, "服务器正在重启，请稍后重新连接。", msg));
        }
        let ids: Vec<u64> = st.clients.keys().copied().collect();
        let reason = bi("服务器重启", msg);
        for id in ids {
            self.remove(&mut st, id, &reason, "left");
        }
    }

    /// Prunes expired history, bans and rate-limit records; drops idle
    /// sessions. Call it every minute.
    pub fn janitor(&self) {
        let now = Instant::now();
        let wall = Utc::now();
        let mut st = self.lock();
        let st = &mut *st;
        for r in st.rooms.values_mut() {
            prune_history(&mut r.history, self.cfg.history_ttl, wall);
        }
        let before = st.bans.len();
        st.bans.retain(|_, until| *until > wall);
        if st.bans.len() != before {
            st.ban_ver += 1;
        }
        for (limit, bots) in [(self.cfg.idle_kick, false), (self.cfg.bot_idle_kick, true)] {
            if limit.is_zero() {
                continue;
            }
            let idle: Vec<u64> = st
                .clients
                .iter()
                .filter(|(_, c)| c.bot == bots && !c.admin && now.duration_since(c.last_active) > limit)
                .map(|(id, _)| *id)
                .collect();
            let (d, d_en) = dur_text(limit);
            let reason = bi(&format!("{d} 未发言，已断开"), &format!("idle for {d_en}"));
            for id in idle {
                self.remove(st, id, &reason, "timed out");
            }
        }
        st.held.retain(|_, (_, until)| *until > now);
        st.conn_log.retain(|_, log| {
            prune_times(log, now - Duration::from_secs(60));
            !log.is_empty()
        });
    }

    /// Processes one line of user input (a message or a /command).
    pub fn handle(&self, id: u64, line: &str) -> Outcome {
        let line = line.trim();
        if line.is_empty() {
            return Outcome::default();
        }
        let mut st = self.lock();
        match st.clients.get_mut(&id) {
            Some(c) => c.last_active = Instant::now(),
            None => return Outcome { quit: true, clear: false },
        }
        if line.starts_with('/') && !line.starts_with("//") {
            return self.command(&mut st, id, line);
        }
        let line = line.strip_prefix('/').filter(|_| line.starts_with("//")).unwrap_or(line); // "//text" sends "/text"
        self.say(&mut st, id, line, Kind::Chat);
        Outcome::default()
    }

    fn command(&self, st: &mut State, id: u64, line: &str) -> Outcome {
        let body = &line[1..];
        let (cmd, rest) = body.split_once(' ').unwrap_or((body, ""));
        let cmd = cmd.to_lowercase();
        let rest = rest.trim();
        match cmd.as_str() {
            "quit" | "exit" | "q" | "bye" => return Outcome { quit: true, clear: false },
            "clear" | "cls" => return Outcome { quit: false, clear: true },
            _ => {}
        }
        let unknown = |st: &State| {
            let c = &st.clients[&id];
            err!(c, format!("没有 /{cmd} 这个命令，输入 /help 查看。"), format!("Unknown command /{cmd} — try /help"));
        };
        match cmd.as_str() {
            "help" | "h" | "?" => self.help(st, id, rest),
            "rooms" | "list" => self.rooms_cmd(st, id),
            "join" | "j" => self.cmd_join(st, id, rest),
            "create" | "new" => self.cmd_create(st, id),
            "leave" | "part" | "lobby" => {
                let lobby = st.public[0].clone();
                let c = &st.clients[&id];
                if c.room == lobby {
                    err!(c, format!("你已经在 #{lobby} 了。"), format!("You are already in #{lobby}."));
                } else {
                    self.join(st, id, &lobby);
                }
            }
            "msg" | "m" | "w" | "tell" | "dm" => {
                let (target, text) = rest.split_once(' ').unwrap_or((rest, ""));
                self.dm(st, id, target.trim_start_matches('@'), text);
            }
            "r" | "reply" => {
                let last = st.clients[&id].last_dm_from.clone();
                if last.is_empty() {
                    err!(st.clients[&id], "还没有人私信你。", "Nobody has messaged you yet.");
                } else {
                    self.dm(st, id, &last, rest);
                }
            }
            "nick" | "name" => self.nick_cmd(st, id, rest),
            "who" | "users" | "names" => self.who(st, id),
            "me" => self.say(st, id, rest, Kind::Action),
            "ignore" | "block" => self.ignore(st, id, rest, true),
            "unignore" | "unblock" => self.ignore(st, id, rest, false),
            "report" => {
                let (target, reason) = rest.split_once(' ').unwrap_or((rest, ""));
                self.report(st, id, target.trim_start_matches('@'), reason);
            }
            "bell" => self.bell(st, id, rest),
            "privacy" | "policy" => self.privacy(st, id),
            "forget" => self.forget(st, id),
            "tz" | "time" => self.tz(st, id, rest),
            "theme" | "bg" | "color" | "colour" => self.theme_cmd(st, id, rest),
            "whoami" => {
                let c = &st.clients[&id];
                let label = room_label(st, &c.room);
                info!(c, format!("你是 {}，在 {label}。", c.nick), format!("You are {} in {label}.", c.nick));
            }
            "kick" | "mute" | "unmute" | "ban" | "unbanall" | "bans" | "unban" | "broadcast" | "announce" | "topic" | "reports" => {
                if st.clients[&id].admin {
                    self.admin(st, id, &cmd, rest);
                } else {
                    unknown(st);
                }
            }
            _ => unknown(st),
        }
        Outcome::default()
    }

    fn help(&self, st: &State, id: u64, arg: &str) {
        let c = &st.clients[&id];
        let a = arg.trim().to_lowercase();
        if a != "all" && a != "more" && a != "全部" {
            let mut ev = Event::new(Kind::Info, "常用命令（直接打字就是发言）", "Everyday commands (just type to talk)");
            ev.lines = help_lines(&[
                ("/rooms", "有哪些房间", "list rooms"),
                ("/join <房间 room>", "进入房间", "join a room"),
                ("/msg <昵称 nick> <内容 text>", "私聊", "private message"),
                ("/nick <名字 name>", "改昵称", "change nickname"),
                ("/who", "谁在这里", "who is here"),
                ("/theme", "换背景：护眼绿（默认）、黑、白", "background: green, black, white"),
                ("/quit", "退出", "leave"),
                ("/help all", "全部命令", "all commands"),
            ]);
            c.send_ev(ev);
            return;
        }
        let mut rows: Vec<(&str, &str, &str)> = vec![
            ("", "聊天", "chat"),
            ("/msg <昵称 nick> <内容 text>", "私聊", "private message"),
            ("/r <内容 text>", "回复上一个私信你的人", "reply to the last private message"),
            ("/me <动作 action>", "动作消息，如 /me 挥手", "action message, e.g. /me waves"),
            ("/nick <名字 name>", "改昵称（2–16 字，中文可以）", "change nickname (2–16 characters)"),
            ("//text", "发送以 / 开头的文字", "send text starting with /"),
            ("", "房间", "rooms"),
            ("/rooms", "房间列表；房间栏 +N 未读，@N 有人 @你", "room list; the bar shows +N unread, @N mentions"),
            ("/join <房间 room>  /leave", "进入、离开房间", "join or leave a room"),
            ("/create", "私人房间，凭房间码进入，没人时自动销毁", "private room, joined by code, gone when empty"),
            ("/who", "当前房间的人", "who is in this room"),
            ("", "隐私与安全", "privacy & safety"),
            ("/privacy", "本服务器存什么、不存什么", "what this server keeps and doesn't"),
            ("/forget", "从服务器删除你发过的消息", "erase your messages from the server"),
            ("/ignore <昵称 nick>", "屏蔽某人（/unignore 取消）", "hide someone (/unignore to undo)"),
            ("/report <昵称 nick> <原因 why>", "举报给管理员", "report to moderators"),
            ("", "其他", "other"),
            ("/tz <时区 zone>", "按你的时区显示时间，如 /tz +8", "show times in your timezone, e.g. /tz -5"),
            ("/bell on|off", "被 @ 或收到私聊时响铃（默认开）", "bell on mentions and private messages (default on)"),
            (
                "/theme green|black|white",
                "背景色：护眼绿（默认）、黑、白；不带参数则轮换",
                "background: eye-care green (default), black, white; no argument cycles",
            ),
            ("/clear  /quit", "清屏、退出", "clear the screen, leave"),
        ];
        if c.admin {
            rows.extend([
                ("", "管理员", "admin"),
                ("/kick <昵称 nick> [原因 reason]", "踢出", "kick"),
                ("/mute <昵称 nick> [分钟 min]", "禁言（/unmute 解除）", "mute (/unmute to undo)"),
                ("/ban <昵称 nick> [分钟 min]", "按地址封禁", "ban by address"),
                ("/bans  /unban <id>  /unbanall", "封禁列表、解封", "list and lift bans"),
                ("/broadcast <内容 text>", "全服公告", "announcement to everyone"),
                ("/topic <内容 text>", "设置房间主题", "set the room topic"),
                ("/reports", "查看举报", "view reports"),
            ]);
        }
        let mut ev = Event::new(Kind::Info, "全部命令", "All commands");
        ev.lines = help_lines(&rows);
        c.send_ev(ev);
    }

    fn rooms_cmd(&self, st: &State, id: u64) {
        let c = &st.clients[&id];
        let mut lines = Vec::new();
        for name in &st.public {
            let r = &st.rooms[name];
            let mark = if c.room == *name { "▸ " } else { "  " };
            let mut extra = String::new();
            let n = r.seq - c.seen.get(name).copied().unwrap_or(0);
            if c.room != *name && n > 0 {
                extra = format!("  （{n} 条新消息 new");
                if c.mentioned.contains(name) {
                    extra.push_str("，有人 @你 · @you");
                }
                extra.push('）');
            }
            lines.push(format!("{mark}#{name:<12} {:>3}   {}{extra}", r.members.len(), r.topic_bi()));
        }
        if let Some(r) = st.rooms.get(&c.room).filter(|r| r.private) {
            lines.push(format!("▸ {}（私人房间 · private）{}", room_label(st, &c.room), r.members.len()));
        }
        let mut ev =
            Event::new(Kind::Info, format!("房间 · 共 {} 人在线", st.clients.len()), format!("Rooms · {} online", st.clients.len()));
        ev.lines = lines;
        c.send_ev(ev);
    }

    fn cmd_join(&self, st: &mut State, id: u64, name: &str) {
        let name = name.trim().trim_start_matches(['#', '~']).to_lowercase();
        let c = &st.clients[&id];
        if name.is_empty() {
            err!(c, "用法：/join <房间>，例如 /join chinese", "Usage: /join <room>, e.g. /join english");
            return;
        }
        if !st.rooms.contains_key(&name) {
            err!(
                c,
                format!("没有 {} 这个房间。/rooms 查看公共房间，/create 创建私人房间。", q(&name)),
                format!("No such room {}. /rooms lists public rooms; /create makes a private one.", q(&name))
            );
            return;
        }
        if c.room == name {
            let l = room_label(st, &name);
            err!(c, format!("你已经在 {l} 了。"), format!("You are already in {l}."));
            return;
        }
        self.join(st, id, &name);
    }

    fn cmd_create(&self, st: &mut State, id: u64) {
        let n = st.rooms.values().filter(|r| r.private).count();
        if n >= self.cfg.max_private {
            err!(st.clients[&id], "现在私人房间太多了，请稍后再试。", "Too many private rooms right now, try later.");
            return;
        }
        if !self.take_token(st.clients.get_mut(&id).unwrap(), 3.0) {
            return;
        }
        let code = loop {
            let code = format!("p-{}", random_code(10));
            if !st.rooms.contains_key(&code) {
                break code;
            }
        };
        st.rooms.insert(code.clone(), Room::new(&code, "", "", true));
        self.join(st, id, &code);
        let c = &st.clients[&id];
        info!(
            c,
            format!("私人房间已创建。把这个房间码发给朋友：/join {code}"),
            format!("Private room created. Give your friends this code: /join {code}")
        );
        info!(c, "最后一个人离开后，房间连同聊天记录一起消失。", "The room disappears (with its history) when the last person leaves.");
    }

    fn join(&self, st: &mut State, id: u64, name: &str) {
        let (old, nick) = {
            let c = &st.clients[&id];
            (c.room.clone(), c.nick.clone())
        };
        if let Some(r) = st.rooms.get_mut(&old) {
            let seq = r.seq;
            r.members.remove(&id);
            st.clients.get_mut(&id).unwrap().seen.insert(old.clone(), seq);
            self.presence(st, &old, &nick, "left for another room", true);
            self.gc_room(st, &old);
        }
        let wall = Utc::now();
        let ttl = self.cfg.history_ttl;
        let limit = self.history_limit(&st.rooms[name]);
        let r = st.rooms.get_mut(name).unwrap();
        prune_history(&mut r.history, ttl, wall);
        let seq = r.seq;
        {
            let c = st.clients.get_mut(&id).unwrap();
            c.room = name.to_string();
            c.seen.insert(name.to_string(), seq);
            c.mentioned.remove(name);
        }
        let r = &st.rooms[name];
        let c = &st.clients[&id];
        let skip = r.history.len().saturating_sub(limit);
        let batch: Vec<Arc<Event>> = r
            .history
            .iter()
            .skip(skip)
            .filter(|ev| !c.ignores.contains(&ev.sender))
            .map(|ev| {
                let mut e = (**ev).clone();
                e.history = true;
                Arc::new(e)
            })
            .collect();
        if !batch.is_empty() {
            let mut ev = Event::new(Kind::History, "", "");
            ev.room = name.to_string();
            ev.history = true;
            ev.batch = batch;
            c.send_ev(ev);
        }
        self.presence(st, name, &nick, "joined", true); // tell the others; the joiner gets "Now in …"
        let r = st.rooms.get_mut(name).unwrap();
        r.members.insert(id);
        let r = &st.rooms[name];
        let (mut topic, mut topic_en) = (r.topic.clone(), r.topic_en.clone());
        if topic_en.is_empty() {
            topic_en = topic.clone();
        }
        if r.private {
            topic = "私人房间".into();
            topic_en = "private room".into();
        }
        let label = room_label(st, name);
        let mut ev = Event::new(
            Kind::Info,
            format!("已进入 {label} · {topic} · {} 人", r.members.len()),
            format!("Now in {label} · {topic_en} · {} here", r.members.len()),
        );
        ev.room = name.to_string();
        // An empty screen looks like something went wrong: say what's going on.
        if r.members.len() == 1 && !r.private {
            ev.lines = vec![others_hint(st, name)];
        } else if r.history.is_empty() {
            ev.lines = vec![bi("这里还没有消息，说第一句吧", "no messages yet — say something")];
        }
        st.clients[&id].send_ev(ev);
    }

    fn gc_room(&self, st: &mut State, name: &str) {
        if st.rooms.get(name).is_some_and(|r| r.private && r.members.is_empty()) {
            st.rooms.remove(name);
        }
    }

    fn history_limit(&self, r: &Room) -> usize {
        if r.private { self.cfg.private_history_size } else { self.cfg.history_size }
    }

    fn say(&self, st: &mut State, id: u64, text: &str, kind: Kind) {
        let text = sanitize(text, self.cfg.max_msg_runes);
        let c = st.clients.get_mut(&id).unwrap();
        if text.is_empty() {
            if kind == Kind::Action {
                err!(c, "用法：/me <动作>", "Usage: /me <action>");
            }
            return;
        }
        if self.muted(c) || !self.take_token(c, 1.0) || self.repeat(c, &text) {
            return;
        }
        let (nick, room) = (c.nick.clone(), c.room.clone());
        st.event_id += 1;
        let ev = Arc::new(Event {
            id: st.event_id,
            time: Utc::now(),
            kind,
            room: room.clone(),
            from: nick,
            text,
            sender: id,
            ..Default::default()
        });
        let limit = self.history_limit(&st.rooms[&room]);
        let r = st.rooms.get_mut(&room).unwrap();
        r.seq += 1;
        r.history.push_back(ev.clone());
        while r.history.len() > limit {
            r.history.pop_front();
        }
        let r = &st.rooms[&room];
        for m in &r.members {
            if let Some(mc) = st.clients.get(m)
                && !mc.ignores.contains(&id)
            {
                mc.send(ev.clone());
            }
        }
        if !r.private {
            // never reveal a private room to someone outside it
            self.notify_mentions(st, id, &room, &ev);
        }
    }

    /// Tells people @mentioned in a room they aren't in. Only the @words in
    /// the text are looked up, so a message without mentions costs nothing
    /// however many users are online.
    fn notify_mentions(&self, st: &mut State, id: u64, room: &str, ev: &Event) {
        let mut notified: HashSet<u64> = HashSet::new();
        let mut rest = ev.text.as_str();
        while notified.len() < 5 {
            let Some(i) = rest.find('@') else { break };
            rest = &rest[i + 1..];
            // Chinese text often has no space after a name ("@夜猫子你好"), so
            // take the longest prefix (max 16 chars) that is an online nick.
            let cand: Vec<char> =
                rest.chars().take_while(|&r| !(r == '@' || is_space(r) || (is_punct(r) && r != '_' && r != '-'))).take(16).collect();
            let mut target = None;
            for n in (2..=cand.len()).rev() {
                let s: String = cand[..n].iter().collect::<String>().to_lowercase();
                if let Some(&t) = st.by_nick.get(&s) {
                    target = Some(t);
                    break;
                }
            }
            let Some(t) = target else { continue };
            let in_room = st.rooms.get(room).is_some_and(|r| r.members.contains(&t));
            let tc = st.clients.get_mut(&t).unwrap();
            if t == id || notified.contains(&t) || in_room || tc.ignores.contains(&id) {
                continue;
            }
            notified.insert(t);
            tc.mentioned.insert(room.to_string());
            tc.send_ev(Event {
                time: ev.time,
                kind: Kind::Mention,
                room: room.to_string(),
                from: ev.from.clone(),
                text: ev.text.clone(),
                ..Default::default()
            });
        }
    }

    fn dm(&self, st: &mut State, id: u64, target: &str, text: &str) {
        let text = sanitize(text, self.cfg.max_msg_runes);
        let c = &st.clients[&id];
        if target.is_empty() || text.is_empty() {
            err!(c, "用法：/msg <昵称> <内容>", "Usage: /msg <nick> <message>");
            return;
        }
        let Some(&to) = st.by_nick.get(&target.to_lowercase()) else {
            err!(c, format!("{} 不在线。", q(target)), format!("No user named {} online.", q(target)));
            return;
        };
        if to == id {
            err!(c, "在跟自己说话？😄", "Talking to yourself? 😄");
            return;
        }
        let c = st.clients.get_mut(&id).unwrap();
        if self.muted(c) || !self.take_token(c, 1.0) || self.repeat(c, &text) {
            return;
        }
        let from = c.nick.clone();
        let now = Utc::now();
        let tc = st.clients.get_mut(&to).unwrap();
        let to_nick = tc.nick.clone();
        let mk = |kind| Event { time: now, kind, from: from.clone(), to: to_nick.clone(), text: text.clone(), ..Default::default() };
        if !tc.ignores.contains(&id) {
            // ignored senders aren't told, so ignoring can't be probed
            tc.last_dm_from = from.clone();
            tc.send_ev(mk(Kind::DmIn));
        }
        st.clients[&id].send_ev(mk(Kind::DmOut));
    }

    fn nick_cmd(&self, st: &mut State, id: u64, n: &str) {
        let n = sanitize(n, 32);
        let c = &st.clients[&id];
        if n.is_empty() {
            info!(c, format!("你现在叫 {}。用法：/nick <新名字>", c.nick), format!("You are {}. Usage: /nick <new name>", c.nick));
            return;
        }
        if !valid_nick(&n) {
            err!(c, "昵称要 2–16 个字，可用中文、字母、数字、_ 和 -。", "Nicknames are 2–16 letters, digits, _ or -.");
            return;
        }
        if reserved_nick(&n) && !c.admin {
            err!(c, format!("{} 是保留名字，换一个吧。", q(&n)), format!("{} is reserved.", q(&n)));
            return;
        }
        if n == c.nick {
            return;
        }
        let lower = n.to_lowercase();
        let held_by_other = st.held.get(&lower).is_some_and(|(k, until)| *k != c.ip_key && *until > Instant::now());
        if st.by_nick.get(&lower).is_some_and(|&o| o != id) || (held_by_other && !c.admin) {
            err!(c, format!("{} 已经有人用了。", q(&n)), format!("{} is taken.", q(&n)));
            return;
        }
        if self.muted(c) {
            return;
        }
        if let Some(at) = c.nick_at
            && !c.admin
        {
            let ready = at + self.cfg.nick_cooldown;
            let now = Instant::now();
            if ready > now {
                let s = (ready - now).as_secs() + 1;
                err!(c, format!("请过 {s} 秒再改名。"), format!("Wait {s}s before changing nick again."));
                return;
            }
        }
        let c = st.clients.get_mut(&id).unwrap();
        let old = std::mem::replace(&mut c.nick, n.clone());
        c.nick_at = Some(Instant::now());
        let room = c.room.clone();
        st.by_nick.remove(&old.to_lowercase());
        st.by_nick.insert(n.to_lowercase(), id);
        st.held.remove(&n.to_lowercase());
        for o in st.clients.values_mut() {
            // keep /r working across renames
            if o.last_dm_from == old {
                o.last_dm_from = n.clone();
            }
        }
        self.room_system(st, &room, &format!("{old} 改名为 {n}"), "renamed");
    }

    fn who(&self, st: &State, id: u64) {
        let c = &st.clients[&id];
        let r = &st.rooms[&c.room];
        let mut names: Vec<&str> = r.members.iter().filter_map(|m| st.clients.get(m)).map(|m| m.nick.as_str()).collect();
        names.sort_by_key(|n| n.to_lowercase());
        let more = names.len().saturating_sub(120);
        names.truncate(120);
        let mut lines: Vec<String> = names.chunks(6).map(|ch| format!("  {}", ch.join("  "))).collect();
        if more > 0 {
            lines.push(format!("  … +{more}"));
        }
        let label = room_label(st, &c.room);
        let mut ev = Event::new(
            Kind::Info,
            format!("{label} 里有 {} 人 · 共 {} 人在线", r.members.len(), st.clients.len()),
            format!("{} in {label} · {} online total", r.members.len(), st.clients.len()),
        );
        ev.lines = lines;
        c.send_ev(ev);
    }

    fn admin(&self, st: &mut State, id: u64, cmd: &str, rest: &str) {
        let (arg, tail) = rest.split_once(' ').unwrap_or((rest, ""));
        let tail = tail.trim();
        match cmd {
            "broadcast" | "announce" => {
                let text = sanitize(rest, self.cfg.max_msg_runes);
                let c = &st.clients[&id];
                if text.is_empty() {
                    err!(c, "用法：/broadcast <内容>", "Usage: /broadcast <text>");
                    return;
                }
                st.event_id += 1;
                let ev = Arc::new(Event {
                    id: st.event_id,
                    time: Utc::now(),
                    kind: Kind::Notice,
                    from: st.clients[&id].nick.clone(),
                    text,
                    ..Default::default()
                });
                for o in st.clients.values() {
                    o.send(ev.clone());
                }
                return;
            }
            "unbanall" => {
                let n = st.bans.len();
                st.bans.clear();
                st.ban_ver += 1;
                info!(st.clients[&id], format!("已清除 {n} 条封禁。"), format!("Cleared {n} bans."));
                return;
            }
            "bans" => {
                let c = &st.clients[&id];
                if st.bans.is_empty() {
                    info!(c, "当前没有封禁。", "No active bans.");
                    return;
                }
                let mut keys: Vec<(&String, &DateTime<Utc>)> = st.bans.iter().collect();
                keys.sort_by_key(|(_, t)| **t);
                let now = Utc::now();
                let lines = keys
                    .iter()
                    .map(|(k, t)| {
                        let left = (**t - now).num_seconds().max(0) as u64;
                        let mins = (left + 30) / 60 * 60;
                        format!("  {}   {}", ban_id(k), go_duration(Duration::from_secs(mins)))
                    })
                    .collect();
                let mut ev = Event::new(
                    Kind::Info,
                    format!("{} 条封禁（id 是匿名地址哈希，后面是剩余时间）· /unban <id>", keys.len()),
                    format!("{} active bans (id = anonymous address hash, then time left) · /unban <id>", keys.len()),
                );
                ev.lines = lines;
                c.send_ev(ev);
                return;
            }
            "unban" => {
                let want = rest.trim().to_lowercase();
                let key = st.bans.keys().find(|k| !want.is_empty() && ban_id(k) == want).cloned();
                let c = &st.clients[&id];
                match key {
                    Some(k) => {
                        st.bans.remove(&k);
                        st.ban_ver += 1;
                        info!(st.clients[&id], format!("已解封 {want}。"), format!("Unbanned {want}."));
                    }
                    None => {
                        err!(c, format!("没有 id 为 {} 的封禁，见 /bans", q(&want)), format!("No ban with id {} — see /bans", q(&want)))
                    }
                }
                return;
            }
            "reports" => {
                let c = &st.clients[&id];
                if st.reports.is_empty() {
                    info!(c, "暂无举报。", "No reports.");
                    return;
                }
                let n = st.reports.len();
                let mut ev = Event::new(Kind::Info, format!("最近 {n} 条举报"), format!("{n} recent reports"));
                ev.lines = st.reports.iter().cloned().collect();
                c.send_ev(ev);
                return;
            }
            "topic" => {
                let room = st.clients[&id].room.clone();
                let r = st.rooms.get_mut(&room).unwrap();
                r.topic = sanitize(rest, 80);
                r.topic_en = r.topic.clone();
                let t = r.topic.clone();
                self.room_system(st, &room, &format!("房间主题改为：{t}"), "topic changed");
                return;
            }
            _ => {}
        }
        let target = st.by_nick.get(&arg.trim_start_matches('@').to_lowercase()).copied();
        let c = &st.clients[&id];
        let Some(t) = target else {
            err!(c, format!("用法：/{cmd} <昵称> …（找不到这个人）"), format!("Usage: /{cmd} <nick> … (user not found)"));
            return;
        };
        if st.clients[&t].admin && t != id {
            err!(c, format!("不能对另一位管理员用 /{cmd}。"), format!("Cannot /{cmd} another admin."));
            return;
        }
        let minutes: i64 = leading_int(tail);
        match cmd {
            "kick" => {
                let reason = if tail.is_empty() {
                    bi("被管理员踢出", "kicked by admin")
                } else {
                    format!("{}：{}", bi("被踢出", "kicked"), sanitize(tail, 120))
                };
                self.remove(st, t, &reason, "was kicked");
            }
            "mute" => {
                let minutes = if minutes <= 0 { 10 } else { minutes };
                let tc = st.clients.get_mut(&t).unwrap();
                tc.muted_until = Some(Instant::now() + Duration::from_secs(minutes as u64 * 60));
                err!(tc, format!("你被管理员禁言 {minutes} 分钟。"), format!("You were muted for {minutes} min by an admin."));
                let tn = tc.nick.clone();
                info!(st.clients[&id], format!("已禁言 {tn} {minutes} 分钟。"), format!("Muted {tn} for {minutes} min."));
            }
            "unmute" => {
                let tc = st.clients.get_mut(&t).unwrap();
                tc.muted_until = None;
                tc.violations = 0;
                info!(tc, "你已解除禁言。", "You can talk again.");
                let tn = tc.nick.clone();
                info!(st.clients[&id], format!("已解除 {tn} 的禁言。"), format!("Unmuted {tn}."));
            }
            "ban" => {
                let key = st.clients[&t].ip_key.clone();
                if key == ONION_KEY {
                    // banning the shared Tor key would lock out every Tor user
                    self.remove(st, t, &bi("被管理员踢出", "kicked by admin"), "was kicked");
                    info!(
                        st.clients[&id],
                        format!("{arg} 通过 Tor 连接，无法按地址封禁，已改为踢出。屡犯可用 /mute。"),
                        format!("{arg} came via Tor and can't be banned by address — kicked instead. Use /mute for repeat offenders.")
                    );
                    return;
                }
                let minutes = if minutes <= 0 { 60 } else { minutes };
                st.bans.insert(key.clone(), Utc::now() + chrono::Duration::minutes(minutes));
                st.ban_ver += 1;
                let victims: Vec<u64> = st.clients.iter().filter(|(_, o)| o.ip_key == key && !o.admin).map(|(i, _)| *i).collect();
                let reason = bi(&format!("被封禁 {minutes} 分钟"), &format!("banned for {minutes} min"));
                for v in victims {
                    self.remove(st, v, &reason, "was banned");
                }
                info!(
                    st.clients[&id],
                    format!("已封禁 {arg} 的地址 {minutes} 分钟。"),
                    format!("Banned {arg}'s address for {minutes} min.")
                );
            }
            _ => {}
        }
    }

    fn ignore(&self, st: &mut State, id: u64, nick: &str, on: bool) {
        let nick = nick.trim().trim_start_matches('@');
        let c = &st.clients[&id];
        if nick.is_empty() {
            let mut names: Vec<&str> = c.ignores.iter().filter_map(|i| st.clients.get(i)).map(|o| o.nick.as_str()).collect();
            if names.is_empty() {
                info!(c, "你没有屏蔽任何人。用法：/ignore <昵称>", "You aren't ignoring anyone. Usage: /ignore <nick>");
            } else {
                names.sort();
                let l = names.join(", ");
                info!(c, format!("已屏蔽：{l}"), format!("Ignoring: {l}"));
            }
            return;
        }
        let Some(&t) = st.by_nick.get(&nick.to_lowercase()) else {
            err!(c, format!("{} 不在线。", q(nick)), format!("No user named {} online.", q(nick)));
            return;
        };
        if t == id {
            err!(c, "不能屏蔽自己。", "You can't ignore yourself.");
            return;
        }
        let tn = st.clients[&t].nick.clone();
        let c = st.clients.get_mut(&id).unwrap();
        if on {
            c.ignores.insert(t);
            info!(
                c,
                format!("已屏蔽 {tn}，看不到对方的消息（对方重连后失效）。"),
                format!("Ignoring {tn} — you won't see their messages (until they reconnect).")
            );
        } else {
            c.ignores.remove(&t);
            info!(c, format!("已取消屏蔽 {tn}。"), format!("No longer ignoring {tn}."));
        }
    }

    fn report(&self, st: &mut State, id: u64, nick: &str, reason: &str) {
        let reason = sanitize(reason, 200);
        let target = st.by_nick.get(&nick.to_lowercase()).copied();
        let c = st.clients.get_mut(&id).unwrap();
        let Some(t) = target.filter(|_| !nick.is_empty()) else {
            err!(c, "用法：/report <昵称> <原因>（对方须在线）", "Usage: /report <nick> <reason> (user must be online)");
            return;
        };
        if !self.take_token(c, 3.0) {
            return;
        }
        let tc = &st.clients[&t];
        let line = format!(
            "[{}] {} reported {} in {}: {reason}",
            Utc::now().format("%m-%d %H:%MZ"),
            st.clients[&id].nick,
            tc.nick,
            room_label(st, &tc.room)
        );
        st.reports.push_back(line.clone());
        while st.reports.len() > 50 {
            st.reports.pop_front();
        }
        let mut admins = 0;
        for o in st.clients.values().filter(|o| o.admin) {
            admins += 1;
            let mut ev = Event::new(Kind::Notice, "举报", "Report");
            ev.lines = vec![line.clone()];
            o.send_ev(ev);
        }
        let c = &st.clients[&id];
        if admins > 0 {
            info!(c, format!("已通知 {admins} 位管理员。"), format!("Report sent to {admins} moderator(s)."));
        } else {
            info!(c, "已记录，管理员上线后可见。", "Report saved; no moderator is online right now.");
        }
    }

    fn tz(&self, st: &mut State, id: u64, arg: &str) {
        let c = st.clients.get_mut(&id).unwrap();
        if arg.trim().is_empty() {
            let now = c.loc.at(Utc::now()).dt.format("%H:%M");
            info!(
                c,
                format!("时间按 {} 显示（现在 {now}）。用法：/tz Asia/Shanghai、/tz +8、/tz utc", c.loc),
                format!("Times are shown in {} (now {now}). Usage: /tz Europe/Berlin | /tz America/New_York | /tz -5 | /tz utc", c.loc)
            );
            return;
        }
        match parse_zone(arg) {
            Ok(loc) => {
                c.loc = loc;
                let now = loc.at(Utc::now()).dt.format("%H:%M");
                info!(c, format!("时区已设为 {loc}，现在 {now}。"), format!("Timezone set to {loc} — now {now}."));
            }
            Err(_) => {
                let a = q(&sanitize(arg, 40));
                err!(
                    c,
                    format!("不认识时区 {a}，例如 /tz Asia/Shanghai 或 /tz +8"),
                    format!("Unknown timezone {a} — e.g. /tz Europe/Berlin or /tz -5")
                );
            }
        }
    }

    fn presence(&self, st: &mut State, room: &str, nick: &str, verb: &str, collapsible: bool) {
        let Some(r) = st.rooms.get_mut(room) else { return };
        if collapsible && self.cfg.quiet_above > 0 && r.members.len() as i64 > self.cfg.quiet_above {
            if verb == "joined" {
                r.joins += 1;
            } else {
                r.leaves += 1;
            }
            return;
        }
        let (zh, en) = match verb {
            "joined" => (format!("{nick} 进入了房间"), "joined"),
            "left" => (format!("{nick} 离开了"), "left"),
            "left for another room" => (format!("{nick} 去了别的房间"), "left for another room"),
            "timed out" => (format!("{nick} 长时间未发言，已断开"), "timed out"),
            "was kicked" => (format!("{nick} 被管理员踢出"), "was kicked"),
            "was banned" => (format!("{nick} 被管理员封禁"), "was banned"),
            v => (format!("{nick} {v}"), v),
        };
        self.room_system(st, room, &zh, en);
    }

    /// Posts the collapsed join/leave summaries of busy rooms. Call it every
    /// 30 seconds.
    pub fn flush_presence(&self) {
        let mut st = self.lock();
        let names: Vec<String> = st.rooms.iter().filter(|(_, r)| r.joins > 0 || r.leaves > 0).map(|(n, _)| n.clone()).collect();
        for name in names {
            let r = st.rooms.get_mut(&name).unwrap();
            let (mut zh, mut en) = (Vec::new(), Vec::new());
            if r.joins > 0 {
                zh.push(format!("{} 人进入", r.joins));
                en.push(format!("{} joined", r.joins));
            }
            if r.leaves > 0 {
                zh.push(format!("{} 人离开", r.leaves));
                en.push(format!("{} left", r.leaves));
            }
            r.joins = 0;
            r.leaves = 0;
            self.room_system(&mut st, &name, &zh.join("、"), &en.join(", "));
        }
    }

    fn bell(&self, st: &mut State, id: u64, arg: &str) {
        let c = st.clients.get_mut(&id).unwrap();
        match arg.trim().to_lowercase().as_str() {
            "off" | "0" | "关" => c.no_bell = true,
            "on" | "1" | "开" => c.no_bell = false,
            "" => c.no_bell = !c.no_bell,
            _ => {
                err!(c, "用法：/bell on 或 /bell off", "Usage: /bell on or /bell off");
                return;
            }
        }
        if c.no_bell {
            info!(c, "响铃已关闭。", "Bell off.");
        } else {
            info!(c, "被 @ 或收到私聊时会响铃。", "Bell on: you'll hear it when mentioned or messaged.");
        }
    }

    fn theme_cmd(&self, st: &mut State, id: u64, arg: &str) {
        let c = st.clients.get_mut(&id).unwrap();
        let next = if arg.trim().is_empty() {
            let i = THEME_ORDER.iter().position(|t| *t == c.theme).unwrap_or(0);
            THEME_ORDER[(i + 1) % THEME_ORDER.len()]
        } else {
            match parse_theme(arg) {
                Some(t) => t,
                None => {
                    err!(
                        c,
                        "用法：/theme black（黑）、/theme white（白）、/theme green（护眼绿）",
                        "Usage: /theme black | /theme white | /theme green"
                    );
                    return;
                }
            }
        };
        c.theme = next;
        let (zh, en) = theme_label(next);
        info!(c, format!("背景已换成{zh}。再输入 /theme 可轮换。"), format!("Background is now {en}. Type /theme again to cycle."));
    }

    /// Applies a theme picked at login (`ssh white@host`); unknown names are
    /// ignored.
    pub fn set_theme(&self, id: u64, name: &str) {
        if let Some(t) = parse_theme(name) {
            let mut st = self.lock();
            if let Some(c) = st.clients.get_mut(&id) {
                c.theme = t;
            }
        }
    }

    /// Marks a session as bot mode (`ssh -T bot@host`): it gets the bot idle
    /// limit. Nothing about it is visible to anyone.
    pub fn mark_bot(&self, id: u64) {
        if let Some(c) = self.lock().clients.get_mut(&id) {
            c.bot = true;
        }
    }

    pub fn bell_on(&self, id: u64) -> bool {
        self.lock().clients.get(&id).is_some_and(|c| !c.no_bell)
    }

    fn privacy(&self, st: &State, id: u64) {
        let cfg = &self.cfg;
        let (bans, bans_en) = if cfg.persist_bans {
            ("已开启持久化：仅被封禁地址的哈希写入磁盘", "persisted: only hashes of banned addresses are written to disk")
        } else {
            ("仅在内存，重启即清空", "memory only, gone on restart")
        };
        let (mut tor, mut tor_en) = if cfg.tor_enabled {
            (
                "已开启，可通过 .onion 地址连接，服务器看不到你的 IP".to_string(),
                "onion service available: connect via .onion and the server never sees your IP".to_string(),
            )
        } else {
            ("未开启洋葱服务".to_string(), "no onion service on this server".to_string())
        };
        if cfg.tor_exits {
            tor.push_str("；用 Tor 浏览器直连也欢迎，Tor 用户共用一组宽松限额");
            tor_en.push_str("; Tor Browser users are welcome and share generous limits");
        }
        let (idle, idle_en) = if cfg.idle_kick.is_zero() {
            ("不会因为不发言而断开".to_string(), "never disconnected for being quiet".to_string())
        } else {
            let (d, d_en) = dur_text(cfg.idle_kick);
            (format!("{d} 不发言自动断开"), format!("disconnected after {d_en} without posting"))
        };
        let (ttl, ttl_en) = dur_text(cfg.history_ttl);
        let hs = cfg.history_size;
        let zh = vec![
            format!("消息   只在内存里，每个房间最多 {hs} 条、保留 {ttl}，然后自动删除；服务器重启即清空；私聊不保存"),
            "不记录 IP、昵称、消息内容都不写入日志；不收集 SSH 公钥；没有账号".into(),
            "地址   只用来限流和封禁，以匿名哈希保存，不显示给任何人（包括管理员）".into(),
            format!("封禁   {bans}"),
            format!("Tor    {tor}"),
            format!("空闲   {idle}"),
            "管理员 可以踢出、禁言、封禁（都会公开显示）；看不到私聊，聊天室里也没有查看 IP 的功能".into(),
            "删除   /forget 立刻从房间历史中删除你本次连接发过的所有消息".into(),
            "边界   SSH 只加密传输：服务器进程能看到明文，VPS 运营者在系统层面能看到连接来源 IP。".into(),
            "       想对运营者也隐藏 IP 请用 Tor；任何时候都不要发送能识别你身份的信息。".into(),
        ];
        let en = vec![
            format!(
                "Messages   memory only, at most {hs} per room, kept {ttl_en}, then deleted; a restart wipes them; private messages are never stored"
            ),
            "No logs    no IPs, nicknames or message text in logs; no SSH public keys collected; no accounts".into(),
            "Addresses  used only for rate limits and bans, kept as anonymous hashes, shown to no one (admins included)".into(),
            format!("Bans       {bans_en}"),
            format!("Tor        {tor_en}"),
            format!("Idle       {idle_en}"),
            "Admins     can kick, mute and ban (always shown publicly); can't read private messages; no IP lookup in the chat".into(),
            "Erase      /forget removes everything you posted this session from room history, now".into(),
            "Limits     SSH encrypts the connection only: the server process sees plaintext and the VPS operator sees source IPs.".into(),
            "           Use Tor to hide your IP from the operator too; never post anything that identifies you.".into(),
        ];
        let mut ev = Event::new(Kind::Info, "本服务器的数据政策", "Privacy on this server");
        ev.lines = zh;
        ev.lines_en = en;
        st.clients[&id].send_ev(ev);
    }

    fn forget(&self, st: &mut State, id: u64) {
        if !self.take_token(st.clients.get_mut(&id).unwrap(), 1.0) {
            return;
        }
        let mut n = 0;
        for r in st.rooms.values_mut() {
            let before = r.history.len();
            r.history.retain(|ev| ev.sender != id);
            n += before - r.history.len();
        }
        info!(
            st.clients[&id],
            format!("已从房间历史删除你的 {n} 条消息；已经显示在别人屏幕上的无法撤回。"),
            format!("Erased {n} of your messages from room history; lines already on others' screens can't be recalled.")
        );
    }

    fn muted(&self, c: &ClientState) -> bool {
        let Some(until) = c.muted_until else { return false };
        let now = Instant::now();
        if now >= until {
            return false;
        }
        let left = go_duration(Duration::from_secs((until - now).as_secs_f64().round() as u64));
        err!(c, format!("你正在禁言中，还剩 {left}。"), format!("You are muted for {left} more."));
        true
    }

    fn take_token(&self, c: &mut ClientState, cost: f64) -> bool {
        if c.admin {
            return true;
        }
        let now = Instant::now();
        c.tokens = self.cfg.msg_burst.min(c.tokens + now.duration_since(c.tokens_at).as_secs_f64() * self.cfg.msg_per_second);
        c.tokens_at = now;
        if c.tokens >= cost {
            c.tokens -= cost;
            return true;
        }
        if c.violation_at.is_none_or(|t| now.duration_since(t) > Duration::from_secs(60)) {
            c.violations = 0;
        }
        c.violations += 1;
        c.violation_at = Some(now);
        if c.violations >= 8 {
            c.violations = 0;
            c.muted_until = Some(now + Duration::from_secs(120));
            err!(c, "刷屏保护：禁言 2 分钟。", "Flood protection: muted for 2 minutes.");
            return false;
        }
        err!(c, "发送太快了，慢一点。", "Slow down — you're sending too fast.");
        false
    }

    fn repeat(&self, c: &mut ClientState, text: &str) -> bool {
        if c.admin {
            return false;
        }
        if text.to_lowercase() == c.last_text.to_lowercase() {
            c.repeat += 1;
        } else {
            c.last_text = text.to_string();
            c.repeat = 0;
        }
        if c.repeat >= 2 {
            err!(c, "请勿重复发送同一条消息。", "Don't repeat the same message.");
            return true;
        }
        false
    }

    fn room_system(&self, st: &mut State, room: &str, text: &str, en: &str) {
        let Some(r) = st.rooms.get(room) else { return };
        st.event_id += 1;
        let ev = Arc::new(Event {
            id: st.event_id,
            time: Utc::now(),
            kind: Kind::System,
            room: room.to_string(),
            text: text.to_string(),
            en: en.to_string(),
            ..Default::default()
        });
        for m in &r.members {
            if let Some(c) = st.clients.get(m) {
                c.send(ev.clone());
            }
        }
    }

    /// The current view of the world for client `id`.
    pub fn status(&self, id: u64) -> Status {
        let st = self.lock();
        let Some(c) = st.clients.get(&id) else { return Status::default() };
        let mut s = Status {
            nick: c.nick.clone(),
            room: c.room.clone(),
            online: st.clients.len(),
            uptime: self.started.elapsed(),
            admin: c.admin,
            muted: c.muted_until.is_some_and(|u| Instant::now() < u),
            loc: c.loc,
            bell: !c.no_bell,
            theme: c.theme,
            room_label: room_label(&st, &c.room),
            ..Default::default()
        };
        if let Some(r) = st.rooms.get(&c.room) {
            s.room_users = r.members.len();
            s.topic = if r.private { bi("私人房间", "private room") } else { r.topic_bi() };
        }
        for name in &st.public {
            let r = &st.rooms[name];
            let current = *name == c.room;
            s.rooms.push(RoomInfo {
                name: name.clone(),
                label: format!("#{name}"),
                users: r.members.len(),
                current,
                private: false,
                unread: if current { 0 } else { r.seq - c.seen.get(name).copied().unwrap_or(0) },
                mentioned: !current && c.mentioned.contains(name),
            });
        }
        if let Some(r) = st.rooms.get(&c.room).filter(|r| r.private) {
            s.rooms.push(RoomInfo {
                name: r.name.clone(),
                label: room_label(&st, &r.name),
                users: r.members.len(),
                current: true,
                private: true,
                unread: 0,
                mentioned: false,
            });
        }
        s
    }

    pub fn nick(&self, id: u64) -> String {
        self.lock().clients.get(&id).map(|c| c.nick.clone()).unwrap_or_default()
    }

    #[cfg(test)]
    pub fn loc(&self, id: u64) -> Zone {
        self.lock().clients.get(&id).map(|c| c.loc).unwrap_or_default()
    }

    /// Online nicknames, for tab completion.
    pub fn nicks(&self) -> Vec<String> {
        self.lock().clients.values().map(|c| c.nick.clone()).collect()
    }

    /// Counts only — no nicknames, addresses or messages.
    pub fn stats(&self) -> Stats {
        let st = self.lock();
        let private = st.rooms.values().filter(|r| r.private).count();
        Stats {
            online: st.clients.len(),
            rooms: st.rooms.len() - private,
            private_rooms: private,
            uptime_seconds: self.started.elapsed().as_secs(),
        }
    }

    /// A copy of the active bans and a version number that changes whenever
    /// they do, so a persister can save only when needed.
    pub fn bans(&self) -> (HashMap<String, DateTime<Utc>>, u64) {
        let st = self.lock();
        let now = Utc::now();
        (st.bans.iter().filter(|(_, v)| **v > now).map(|(k, v)| (k.clone(), *v)).collect(), st.ban_ver)
    }

    /// Installs previously saved bans (expired ones are skipped).
    pub fn load_bans(&self, b: HashMap<String, DateTime<Utc>>) -> usize {
        let mut st = self.lock();
        let now = Utc::now();
        let mut n = 0;
        for (k, v) in b {
            if v > now && k != ONION_KEY {
                st.bans.insert(k, v);
                n += 1;
            }
        }
        n
    }
}

fn help_lines(rows: &[(&str, &str, &str)]) -> Vec<String> {
    rows.iter()
        .map(|(cmd, zh, en)| if cmd.is_empty() { format!("── {zh} · {en} ──") } else { format!("{}{zh} · {en}", pad_right(cmd, 30)) })
        .collect()
}

/// Pads `s` with spaces to `n` terminal columns (CJK counts as 2).
pub fn pad_right(s: &str, n: usize) -> String {
    let w: usize = s
        .chars()
        .map(|r| {
            let u = r as u32;
            if u >= 0x1100
                && (u <= 0x115f
                    || (0x2e80..=0xa4cf).contains(&u)
                    || (0xac00..=0xd7a3).contains(&u)
                    || (0xf900..=0xfaff).contains(&u)
                    || (0xfe30..=0xfe4f).contains(&u)
                    || (0xff00..=0xff60).contains(&u)
                    || (0xffe0..=0xffe6).contains(&u))
            {
                2
            } else {
                1
            }
        })
        .sum();
    if w >= n { format!("{s} ") } else { format!("{s}{}", " ".repeat(n - w)) }
}

fn room_label(st: &State, name: &str) -> String {
    if st.rooms.get(name).is_some_and(|r| r.private) { format!("🔒{name}") } else { format!("#{name}") }
}

/// Tells someone alone in a public room where people are.
fn others_hint(st: &State, here: &str) -> String {
    let (mut best, mut n) = ("", 0);
    for name in &st.public {
        let c = st.rooms[name].members.len();
        if name != here && c > n {
            best = name;
            n = c;
        }
    }
    if best.is_empty() {
        if st.clients.len() > 1 {
            return bi("这里暂时只有你，有人进来会显示在这里", "only you here for now; arrivals show up here");
        }
        return bi("暂时只有你在线，有人进来会显示在这里", "you're the only one online for now; arrivals show up here");
    }
    bi(&format!("这里暂时只有你；#{best} 有 {n} 人，/join {best}"), &format!("only you here; #{best} has {n} — /join {best}"))
}

fn prune_history(h: &mut VecDeque<Arc<Event>>, ttl: Duration, now: DateTime<Utc>) {
    let ttl = chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::MAX);
    while h.front().is_some_and(|ev| now - ev.time > ttl) {
        h.pop_front();
    }
}

fn prune_times(ts: &mut VecDeque<Instant>, after: Instant) {
    while ts.front().is_some_and(|t| *t < after) {
        ts.pop_front();
    }
}

fn ban_id(key: &str) -> String {
    key.chars().take(8).collect()
}

/// `fmt.Sscanf(s, "%d", &n)`: the leading integer of s, or 0.
fn leading_int(s: &str) -> i64 {
    let s = s.trim_start();
    let end =
        s.char_indices().find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))).map(|(i, _)| i).unwrap_or(s.len());
    s[..end].parse().unwrap_or(0)
}

pub(crate) fn random_bytes(buf: &mut [u8]) {
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new().fill(buf).expect("system random number generator failed");
}

fn random_below(n: u64) -> u64 {
    let mut b = [0u8; 8];
    random_bytes(&mut b);
    u64::from_le_bytes(b) % n // n ≤ 10^k for small k: bias is negligible for nicknames
}

fn free_nick(by_nick: &HashMap<String, u64>) -> String {
    let (mut digits, mut span) = (4usize, 10_000u64);
    loop {
        for _ in 0..20 {
            let n = format!("anonymous_{:0digits$}", random_below(span));
            if !by_nick.contains_key(&n) {
                return n;
            }
        }
        digits += 1;
        span *= 10;
    }
}

const CODE_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

pub fn random_code(n: usize) -> String {
    let limit = 256 - 256 % CODE_ALPHABET.len();
    let mut out = String::with_capacity(n);
    let mut buf = vec![0u8; 2 * n];
    while out.len() < n {
        random_bytes(&mut buf);
        for &c in &buf {
            if (c as usize) < limit && out.len() < n {
                out.push(CODE_ALPHABET[c as usize % CODE_ALPHABET.len()] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
