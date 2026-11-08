//! Bot mode (`ssh -T bot@host`): one JSON object per line in both
//! directions, so an AI agent or script never has to parse the
//! human-oriented, bilingual chat lines. In the room it looks like anyone
//! else.
//!
//! Output: see `AgentEvent`. Input, one per line, either
//!
//! ```text
//! plain text                   sent exactly as if typed (commands work: /join tech)
//! {"text":"..."}               a message for the current room; may be long or
//!                              contain newlines: it's split into chat-sized
//!                              messages and paced to the rate limit
//! {"to":"nick","text":"..."}   the same, as a private message
//! {"cmd":"/who"}               one command
//! {"type":"ping"}              answered with {"type":"pong"} (doesn't count as activity)
//! ```

use super::format::mentions;
use super::{Input, Io, done_reason};
use crate::chat::{Client, Event, Hub, Kind};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// How many chat messages one input may turn into, so a runaway model can't
/// flood the room.
pub const MAX_SPLIT: usize = 8;

#[derive(Serialize, Default, Debug)]
pub struct Limits {
    max_runes: usize,
    burst: f64,
    per_second: f64,
    idle_kick: u64,
    max_split: usize,
}

/// One line of bot-mode output.
#[derive(Serialize, Default, Debug)]
pub struct AgentEvent {
    /// hello, chat, action, system, dm, info, error, notice, mention, pong,
    /// disconnect, bye
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "is_zero")]
    pub id: u64,
    #[serde(serialize_with = "ser_time")]
    pub time: DateTime<Utc>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub room: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub from: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub nick: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub en: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines_en: Vec<String>,
    #[serde(rename = "self", skip_serializing_if = "is_false")]
    pub self_: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub mention: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub history: bool,
    #[serde(rename = "incoming", skip_serializing_if = "is_false")]
    pub incoming: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn ser_time<S: serde::Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.with_timezone(&Local).to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Chat => "chat",
        Kind::Action => "action",
        Kind::System => "system",
        Kind::DmIn | Kind::DmOut => "dm",
        Kind::Info => "info",
        Kind::Error => "error",
        Kind::Notice => "notice",
        Kind::Mention => "mention",
        Kind::History => "",
    }
}

fn now_ev(kind: &'static str) -> AgentEvent {
    AgentEvent { kind, time: Utc::now(), ..Default::default() }
}

#[derive(Debug, PartialEq)]
enum AgentInput {
    None,
    /// a line to hand to Hub::handle as typed
    Cmd(String),
    /// chat-sized lines to send through the pacer
    Msgs(Vec<String>),
    Ping,
    Err(String),
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AgentJson {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    to: String,
    cmd: String,
}

fn parse(line: &str, max_runes: usize) -> AgentInput {
    let line = line.trim();
    if line.is_empty() {
        return AgentInput::None;
    }
    if !line.starts_with('{') {
        return AgentInput::Cmd(line.to_string());
    }
    let j: AgentJson = match serde_json::from_str(line) {
        Ok(j) => j,
        Err(e) => return AgentInput::Err(e.to_string()),
    };
    if j.kind == "ping" {
        return AgentInput::Ping;
    }
    if !j.cmd.is_empty() {
        if !j.cmd.starts_with('/') {
            return AgentInput::Err(r#""cmd" must start with "/""#.into());
        }
        return AgentInput::Cmd(j.cmd.replace('\n', " "));
    }
    if j.text.trim().is_empty() {
        return AgentInput::Err(r#"expected "text", "cmd" or "type":"ping""#.into());
    }
    let mut prefix = String::new();
    if !j.to.is_empty() {
        if j.to.contains([' ', '\t', '\r', '\n']) {
            return AgentInput::Err(r#""to" must be one nick"#.into());
        }
        prefix = format!("/msg {} ", j.to);
    }
    let msgs = split_message(&j.text, max_runes, MAX_SPLIT)
        .into_iter()
        .map(|mut part| {
            if prefix.is_empty() && part.starts_with('/') {
                part.insert(0, '/'); // text is never a command: "//x" sends "/x"
            }
            format!("{prefix}{part}")
        })
        .collect();
    AgentInput::Msgs(msgs)
}

/// Turns text into at most `max_parts` chat messages of at most `max_runes`
/// characters each: one per non-empty line, long lines broken at a space
/// where possible. If it doesn't fit, the last part ends with "…".
pub fn split_message(text: &str, max_runes: usize, max_parts: usize) -> Vec<String> {
    let max_runes = max_runes.max(2);
    let mut out: Vec<String> = Vec::new();
    let mut full = false;
    'lines: for line in text.replace("\r\n", "\n").split('\n') {
        let mut line = line.trim();
        while !line.is_empty() {
            if out.len() == max_parts {
                full = true;
                break 'lines;
            }
            if line.chars().count() <= max_runes {
                out.push(line.to_string());
                break;
            }
            let mut cut = line.char_indices().nth(max_runes).map_or(line.len(), |(i, _)| i);
            if let Some(sp) = line[..cut].rfind(' ')
                && sp > cut / 2
            {
                cut = sp;
            }
            out.push(line[..cut].trim().to_string());
            line = line[cut..].trim();
        }
    }
    if full && let Some(last) = out.last_mut() {
        let mut chars: Vec<char> = last.chars().collect();
        if chars.len() >= max_runes {
            chars.truncate(max_runes - 1);
        }
        *last = chars.into_iter().collect::<String>() + "…";
    }
    out
}

struct Agent<'a> {
    hub: &'a Hub,
    id: u64,
    out: String,
}

impl Agent<'_> {
    fn emit(&mut self, e: &AgentEvent) {
        if let Ok(s) = serde_json::to_string(e) {
            self.out.push_str(&s);
            self.out.push('\n');
        }
    }

    fn emit_event(&mut self, ev: &Event) {
        let kind = kind_name(ev.kind);
        if kind.is_empty() {
            return;
        }
        let me = self.hub.nick(self.id);
        let mut e = AgentEvent {
            kind,
            id: ev.id,
            time: ev.time,
            room: ev.room.clone(),
            from: ev.from.clone(),
            to: ev.to.clone(),
            text: ev.text.clone(),
            en: ev.en.clone(),
            lines: ev.lines.clone(),
            lines_en: ev.lines_en.clone(),
            history: ev.history,
            ..Default::default()
        };
        match ev.kind {
            Kind::Chat | Kind::Action => {
                e.self_ = ev.from == me;
                e.mention = !e.self_ && mentions(&ev.text, &me);
            }
            Kind::DmIn => {
                e.incoming = true;
                e.mention = true;
            }
            Kind::DmOut => e.self_ = true,
            Kind::Mention => e.mention = true,
            _ => {}
        }
        self.emit(&e);
    }
}

pub async fn run(io: &mut Io, hub: &Hub, c: &mut Client) {
    let cfg = hub.config();
    let mut a = Agent { hub, id: c.id, out: String::new() };
    let st = hub.status(c.id);
    a.emit(&AgentEvent {
        nick: st.nick,
        room: st.room,
        limits: Some(Limits {
            max_runes: cfg.max_msg_runes,
            burst: cfg.msg_burst,
            per_second: cfg.msg_per_second,
            idle_kick: cfg.bot_idle_kick.as_secs(),
            max_split: MAX_SPLIT,
        }),
        ..now_ev("hello")
    });

    // Outgoing chunks wait here so a long reply doesn't block reading events;
    // the pacer keeps them within the server's rate limit.
    let mut queue: VecDeque<String> = VecDeque::new();
    let rate = cfg.msg_per_second.max(0.01);
    let burst = (cfg.msg_burst - 1.0).max(1.0); // keep one token spare for commands
    let (mut tokens, mut at) = (burst, Instant::now());
    let mut line_buf: Vec<u8> = Vec::new();
    let mut last_cr = false;
    let mut quitting: Option<Instant> = None; // input ended: deadline to send what's queued
    let max_runes = cfg.max_msg_runes;

    loop {
        if !io.write(a.out.as_bytes()).await {
            return;
        }
        a.out.clear();
        if let Some(deadline) = quitting
            && (queue.is_empty() || Instant::now() >= deadline)
        {
            // what our last messages produced (their echo, replies)
            tokio::time::sleep(Duration::from_millis(50)).await;
            while let Ok(ev) = c.rx.try_recv() {
                a.emit_event(&ev);
            }
            let _ = io.write(a.out.as_bytes()).await;
            return;
        }
        // when the next queued message may go
        let next_send = if queue.is_empty() {
            None
        } else {
            let now = Instant::now();
            let t = (tokens + now.duration_since(at).as_secs_f64() * rate).min(burst);
            Some(if t >= 1.0 { now } else { now + Duration::from_secs_f64((1.0 - t) / rate + 0.02) })
        };
        tokio::select! {
            ev = c.rx.recv() => {
                let Some(ev) = ev else {
                    a.emit(&AgentEvent { text: done_reason(c).unwrap_or_default(), ..now_ev("disconnect") });
                    let _ = io.write(a.out.as_bytes()).await;
                    return;
                };
                if ev.kind == Kind::History {
                    for e in &ev.batch {
                        a.emit_event(e);
                    }
                } else {
                    a.emit_event(&ev);
                }
            }
            _ = c.done.changed() => {
                while let Ok(ev) = c.rx.try_recv() {
                    a.emit_event(&ev);
                }
                a.emit(&AgentEvent { text: done_reason(c).unwrap_or_default(), ..now_ev("disconnect") });
                let _ = io.write(a.out.as_bytes()).await;
                return;
            }
            _ = async { tokio::time::sleep_until(next_send.unwrap()).await }, if next_send.is_some() => {
                let now = Instant::now();
                tokens = (tokens + now.duration_since(at).as_secs_f64() * rate).min(burst);
                at = now;
                if tokens < 1.0 {
                    tokens = 1.0;
                }
                tokens -= 1.0;
                if let Some(m) = queue.pop_front() {
                    hub.handle(c.id, &m);
                }
            }
            _ = async { tokio::time::sleep_until(quitting.unwrap()).await }, if quitting.is_some() => {}
            inp = io.input.recv(), if quitting.is_none() => {
                let data = match inp {
                    Some(Input::Data(d)) => d,
                    Some(Input::Resize(..)) => continue,
                    Some(Input::Eof) | None => {
                        let mut lines = Vec::new();
                        if !line_buf.is_empty() {
                            lines.push(String::from_utf8_lossy(&std::mem::take(&mut line_buf)).into_owned());
                        }
                        for l in lines {
                            handle_input(&mut a, hub, c.id, parse(&l, max_runes), &mut queue);
                        }
                        // Input ended (`echo ... | ssh -T bot@host`): send what's queued first.
                        quitting = Some(Instant::now() + Duration::from_secs_f64((queue.len() + 1) as f64 / rate + 2.0));
                        continue;
                    }
                };
                for b in data {
                    let cr = last_cr;
                    last_cr = b == b'\r';
                    if b == b'\n' && cr {
                        continue;
                    }
                    if b == b'\n' || b == b'\r' {
                        let l = String::from_utf8_lossy(&std::mem::take(&mut line_buf)).into_owned();
                        if handle_input(&mut a, hub, c.id, parse(&l, max_runes), &mut queue) {
                            let _ = io.write(a.out.as_bytes()).await;
                            return;
                        }
                    } else if line_buf.len() < 256 * 1024 {
                        line_buf.push(b);
                    }
                }
            }
        }
    }
}

/// Returns true when the session should end.
fn handle_input(a: &mut Agent, hub: &Hub, id: u64, inp: AgentInput, queue: &mut VecDeque<String>) -> bool {
    match inp {
        AgentInput::None => {}
        AgentInput::Ping => a.emit(&now_ev("pong")),
        AgentInput::Cmd(cmd) => {
            if hub.handle(id, &cmd).quit {
                a.emit(&now_ev("bye"));
                return true;
            }
        }
        AgentInput::Err(e) => a.emit(&AgentEvent { text: format!("输入无效：{e}"), en: format!("Bad input: {e}"), ..now_ev("error") }),
        AgentInput::Msgs(msgs) => {
            for m in msgs {
                if queue.len() >= 4 * MAX_SPLIT {
                    a.emit(&AgentEvent {
                        text: "发送队列已满，消息被丢弃".into(),
                        en: "Send queue full — message dropped".into(),
                        ..now_ev("error")
                    });
                } else {
                    queue.push_back(m);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::testio;
    use super::*;
    use crate::chat::Config;

    #[test]
    fn splits() {
        assert_eq!(split_message("a\n\nb", 10, 8), vec!["a", "b"]);
        assert_eq!(split_message("hello world foo", 11, 8), vec!["hello world", "foo"]);
        let parts = split_message(&"x".repeat(100), 10, 3);
        assert_eq!(parts.len(), 3);
        assert!(parts[2].ends_with('…') && parts[2].chars().count() == 10);
        assert_eq!(split_message("你好世界", 2, 8), vec!["你好", "世界"]);
    }

    #[test]
    fn parses() {
        assert_eq!(parse("/join tech", 500), AgentInput::Cmd("/join tech".into()));
        assert_eq!(parse(r#"{"type":"ping"}"#, 500), AgentInput::Ping);
        assert_eq!(parse(r#"{"text":"/help"}"#, 500), AgentInput::Msgs(vec!["//help".into()]));
        assert_eq!(parse(r#"{"to":"Alice","text":"hi"}"#, 500), AgentInput::Msgs(vec!["/msg Alice hi".into()]));
        assert!(matches!(parse(r#"{"to":"a b","text":"hi"}"#, 500), AgentInput::Err(_)));
        assert!(matches!(parse(r#"{"cmd":"who"}"#, 500), AgentInput::Err(_)));
        assert!(matches!(parse("{oops", 500), AgentInput::Err(_)));
    }

    #[tokio::test]
    async fn json_session_paces_and_flushes_on_eof() {
        let hub = Hub::new(Config { msg_burst: 3.0, msg_per_second: 20.0, ..Default::default() });
        let mut other = hub.connect("b", false).unwrap();
        let mut me = hub.connect("a", false).unwrap();
        let (mut io, tx, cap) = testio::io();
        let h = hub.clone();
        let task = tokio::spawn(async move { run(&mut io, &h, &mut me).await });
        tx.send(Input::Data(b"{\"cmd\":\"/nick Helper\"}\n{\"text\":\"one\\ntwo\\nthree\\nfour\\nfive\"}\n{\"type\":\"ping\"}\n".to_vec()))
            .await
            .unwrap();
        tx.send(Input::Eof).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        let out = cap.text();
        let first: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
        assert_eq!(first["type"], "hello");
        assert_eq!(first["limits"]["max_split"], 8);
        assert!(out.contains(r#""type":"pong""#));
        assert!(!out.contains("too fast"), "pacing tripped flood protection:\n{out}");
        let mut got = Vec::new();
        while let Ok(ev) = other.rx.try_recv() {
            if ev.kind == Kind::Chat {
                got.push(ev.text.clone());
            }
        }
        assert_eq!(got, ["one", "two", "three", "four", "five"]);
        // our own messages come back marked self
        assert!(out.lines().any(|l| l.contains(r#""text":"five""#) && l.contains(r#""self":true"#)), "{out}");
    }
}
