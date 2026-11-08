//! Line mode: no colours, no cursor movement, no alternate screen. Used for
//! `ssh plain@host`, for `ssh -T` (no PTY) and for TERM=dumb. With a PTY the
//! server echoes keystrokes and redraws the input line itself (the client
//! terminal is in raw mode); without one the client's terminal edits lines.

use super::format::{Formatter, rings};
use super::text::{char_width, width};
use super::{Input, Io, done_reason};
use crate::chat::{Client, Event, Hub, Kind};
use std::time::{Duration, Instant};

/// How much of a room's history line mode prints on join: enough for context
/// without flooding a phone screen (or a script) with a day of chat.
const REPLAY: usize = 50;

struct Plain<'a> {
    hub: &'a Hub,
    id: u64,
    fmt: Formatter,
    pty: bool,
    nl: &'static str,
    width: usize,
    buf: Vec<char>,
    rang: Option<Instant>,
    out: String,
}

pub async fn run(io: &mut Io, hub: &Hub, c: &mut Client, pty: bool, width: usize) {
    let mut p = Plain {
        hub,
        id: c.id,
        fmt: Formatter::new(false),
        pty,
        nl: if pty { "\r\n" } else { "\n" },
        width: if width == 0 { 80 } else { width },
        buf: Vec::new(),
        rang: None,
        out: String::new(),
    };
    let nl = p.nl;
    p.out.push_str(&format!("VIVANT（纯文本模式 plain mode）· /help 帮助 help · /quit 退出 quit{nl}"));
    p.draw_prompt();
    let mut lines = LineReader::new(pty, hub.config().max_msg_runes + 64);
    if !io.write(p.out.as_bytes()).await {
        return;
    }
    p.out.clear();
    loop {
        tokio::select! {
            ev = c.rx.recv() => {
                let Some(ev) = ev else {
                    p.done(c);
                    let _ = io.write(p.out.as_bytes()).await;
                    return;
                };
                p.refresh();
                if ev.kind == Kind::History {
                    p.print_history(&ev.batch);
                } else {
                    p.print_event(&ev);
                    if rings(&ev, &p.fmt.self_nick) && hub.bell_on(c.id) && p.rang.is_none_or(|r| r.elapsed() > Duration::from_secs(2)) {
                        p.out.push('\x07');
                        p.rang = Some(Instant::now());
                    }
                }
            }
            _ = c.done.changed() => {
                p.done(c);
                let _ = io.write(p.out.as_bytes()).await;
                return;
            }
            inp = io.input.recv() => match inp {
                Some(Input::Data(d)) => {
                    for act in lines.feed(&d) {
                        match act {
                            LineAct::Echo(s) => {
                                if p.pty {
                                    p.out.push_str(&s);
                                }
                            }
                            LineAct::Backspace(ch) => {
                                p.buf.pop();
                                let w = char_width(ch).max(1);
                                p.out.push_str(&"\x08".repeat(w));
                                p.out.push_str(&" ".repeat(w));
                                p.out.push_str(&"\x08".repeat(w));
                            }
                            LineAct::Char(ch) => p.buf.push(ch),
                            LineAct::Kill => {
                                p.clear_input();
                                p.buf.clear();
                                p.draw_prompt();
                            }
                            LineAct::Line(line) => {
                                p.buf.clear();
                                if p.pty {
                                    p.out.push_str(nl);
                                }
                                if line.is_empty() {
                                    p.draw_prompt();
                                    continue;
                                }
                                let quit = hub.handle(c.id, &line).quit;
                                p.drain(c);
                                if quit {
                                    p.print_raw("再见！Bye!");
                                    let _ = io.write(p.out.as_bytes()).await;
                                    return;
                                }
                                p.draw_prompt();
                            }
                            LineAct::Quit => {
                                let _ = io.write(p.out.as_bytes()).await;
                                return;
                            }
                        }
                    }
                }
                Some(Input::Resize(w, _)) => p.width = w.max(1),
                Some(Input::Eof) | None => {
                    if let Some(line) = lines.flush() {
                        hub.handle(c.id, &line);
                    }
                    p.drain(c);
                    let _ = io.write(p.out.as_bytes()).await;
                    return;
                }
            },
        }
        if !io.write(p.out.as_bytes()).await {
            return;
        }
        p.out.clear();
    }
}

impl Plain<'_> {
    /// Prints what the hub has queued for us (a command's reply, our own
    /// message), so output follows input in order.
    fn drain(&mut self, c: &mut Client) {
        while let Ok(ev) = c.rx.try_recv() {
            self.refresh();
            if ev.kind == Kind::History {
                self.print_history(&ev.batch);
            } else {
                self.print_event(&ev);
            }
        }
    }

    fn refresh(&mut self) {
        let s = self.hub.status(self.id);
        self.fmt.self_nick = s.nick;
        self.fmt.loc = s.loc;
    }

    fn done(&mut self, c: &Client) {
        if let Some(r) = done_reason(c).filter(|r| !r.is_empty()) {
            self.print_raw(&format!("已断开 Disconnected: {r}"));
        }
    }

    /// Erases the in-progress input line without ANSI sequences.
    fn clear_input(&mut self) {
        if !self.pty {
            return;
        }
        let s: String = self.buf.iter().collect();
        let n = width(&format!("> {s}")) % self.width.max(1);
        self.out.push('\r');
        self.out.push_str(&" ".repeat(n));
        self.out.push('\r');
    }

    fn draw_prompt(&mut self) {
        if self.pty {
            self.out.push_str("> ");
            self.out.extend(self.buf.iter());
        }
    }

    fn print_raw(&mut self, s: &str) {
        self.clear_input();
        self.out.push_str(s);
        self.out.push_str(self.nl);
    }

    fn print_event(&mut self, ev: &Event) {
        self.clear_input();
        for l in self.fmt.lines(ev) {
            self.out.push_str(&l);
            self.out.push_str(self.nl);
        }
        self.draw_prompt();
    }

    fn print_history(&mut self, batch: &[std::sync::Arc<Event>]) {
        let mut batch = batch;
        if batch.len() > REPLAY {
            let n = batch.len() - REPLAY;
            self.print_raw(&format!("（省略更早的 {n} 条 · {n} earlier messages not shown）"));
            batch = &batch[n..];
        }
        for ev in batch {
            self.print_event(ev);
        }
    }
}

pub enum LineAct {
    Echo(String),
    Char(char),
    Backspace(char),
    Kill,
    Line(String),
    Quit,
}

/// Turns raw input into lines. With a PTY: keystrokes, echo and simple
/// editing; without: newline-separated lines (CR, LF or CRLF).
pub struct LineReader {
    pty: bool,
    max: usize,
    pending: Vec<u8>, // partial UTF-8 / escape across packets
    line: Vec<char>,
    raw_line: Vec<u8>, // no-PTY mode
    skip_esc: u8,      // 1 = after ESC, 2 = inside CSI/SS3
    last_cr: bool,
}

/// Longest line accepted without a PTY (like the Go version's 64 KiB
/// scanner buffer); the rest of an overlong line is dropped.
const RAW_MAX: usize = 64 * 1024;

impl LineReader {
    pub fn new(pty: bool, max: usize) -> LineReader {
        LineReader { pty, max, pending: Vec::new(), line: Vec::new(), raw_line: Vec::new(), skip_esc: 0, last_cr: false }
    }

    /// What's left when the input ends without a final newline.
    pub fn flush(&mut self) -> Option<String> {
        if self.pty || self.raw_line.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&std::mem::take(&mut self.raw_line)).into_owned())
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<LineAct> {
        let mut out = Vec::new();
        if !self.pty {
            for &b in data {
                let cr = self.last_cr;
                self.last_cr = b == b'\r';
                if b == b'\n' && cr {
                    continue;
                }
                if b == b'\n' || b == b'\r' {
                    let l = String::from_utf8_lossy(&std::mem::take(&mut self.raw_line)).into_owned();
                    out.push(LineAct::Line(l));
                } else if self.raw_line.len() < RAW_MAX {
                    self.raw_line.push(b);
                }
            }
            return out;
        }
        self.pending.extend_from_slice(data);
        let mut i = 0;
        let buf = std::mem::take(&mut self.pending);
        while i < buf.len() {
            let b = buf[i];
            if self.skip_esc == 1 {
                self.skip_esc = if b == b'[' || b == b'O' { 2 } else { 0 };
                i += 1;
                continue;
            }
            if self.skip_esc == 2 {
                if (0x40..=0x7e).contains(&b) {
                    self.skip_esc = 0;
                }
                i += 1;
                continue;
            }
            match b {
                b'\r' | b'\n' => {
                    let line: String = self.line.drain(..).collect();
                    out.push(LineAct::Line(line));
                }
                0x03 | 0x04 => {
                    out.push(LineAct::Quit);
                    return out;
                }
                0x7f | 0x08 => {
                    if let Some(ch) = self.line.pop() {
                        out.push(LineAct::Backspace(ch));
                    }
                }
                0x15 => {
                    self.line.clear();
                    out.push(LineAct::Kill);
                }
                0x1b => self.skip_esc = 1,
                b if b < 0x20 => {}
                _ => {
                    let len = match b {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        0x20..=0x7f => 1,
                        _ => {
                            i += 1;
                            continue;
                        }
                    };
                    if buf.len() - i < len {
                        self.pending = buf[i..].to_vec();
                        return out;
                    }
                    if let Ok(s) = std::str::from_utf8(&buf[i..i + len]) {
                        let ch = s.chars().next().unwrap();
                        if crate::chat::sanitize::printable(ch) && self.line.len() < self.max {
                            self.line.push(ch);
                            out.push(LineAct::Char(ch));
                            out.push(LineAct::Echo(ch.to_string()));
                        }
                        i += len;
                        continue;
                    }
                }
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::testio;
    use super::*;
    use crate::chat::Config;

    #[tokio::test]
    async fn no_pty_session() {
        let hub = Hub::new(Config { msg_burst: 100.0, msg_per_second: 100.0, ..Default::default() });
        let mut me = hub.connect("a", false).unwrap();
        let (mut io, tx, cap) = testio::io();
        let h = hub.clone();
        let task = tokio::spawn(async move { run(&mut io, &h, &mut me, false, 80).await });
        tx.send(Input::Data(b"/nick ci-bot\nhello from CI\n".to_vec())).await.unwrap();
        tx.send(Input::Data("/tz +8\r\n中文".as_bytes().to_vec())).await.unwrap();
        tx.send(Input::Data("也行\n/quit\n".as_bytes().to_vec())).await.unwrap();
        task.await.unwrap();
        let out = cap.text();
        assert!(out.contains("Welcome"), "{out}");
        assert!(out.contains("<ci-bot> hello from CI"), "{out}");
        assert!(out.contains("<ci-bot> 中文也行"), "{out}");
        assert!(!out.contains('\x1b'), "escape codes in plain mode");
    }

    #[test]
    fn pty_editing() {
        let mut r = LineReader::new(true, 100);
        let acts = r.feed("ab\x7f你\x1b[Dc\r".as_bytes());
        let line = acts.iter().find_map(|a| if let LineAct::Line(l) = a { Some(l.clone()) } else { None });
        assert_eq!(line.as_deref(), Some("a你c"));
    }
}
