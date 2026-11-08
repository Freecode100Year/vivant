//! The full-screen UI: a small purpose-built terminal renderer rather than a
//! general TUI framework. A chat screen is mostly "one new line scrolls in at
//! the bottom", which a terminal does natively with a scroll region
//! (DECSTBM): for each message the server sends just that line. The fixed
//! rows (header, rooms, separator, input, footer) are redrawn only when their
//! content changes, so a busy room costs a few hundred bytes per session per
//! message.
//!
//! Layout (H = terminal height):
//!
//! ```text
//! 1        header bar
//! 2        rooms strip
//! 3..H-3   chat (scroll region)
//! H-2      separator: current room, topic, scroll state
//! H-1      input line
//! H        key hints + connection status
//! ```

use super::format::{Formatter, rings, uptime};
use super::keys::{Decoder, Key};
use super::style::{RESET_DEFAULTS, Theme, theme_by_name};
use super::text::{active_style, truncate, width, wrap};
use super::{Input, Io, done_reason};
use crate::chat::sanitize::printable;
use crate::chat::{Client, Event, Hub, Kind, Status, bi};
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_EVENTS: usize = 500;
const MAX_INPUT_HIST: usize = 50;
const MIN_WIDTH: usize = 20;
const MIN_HEIGHT: usize = 8;

const COMPLETIONS: &[&str] = &[
    "/help",
    "/rooms",
    "/join ",
    "/create",
    "/leave",
    "/msg ",
    "/reply ",
    "/nick ",
    "/who",
    "/me ",
    "/ignore ",
    "/unignore ",
    "/report ",
    "/tz ",
    "/theme ",
    "/clear",
    "/quit",
];

struct Term<'a> {
    hub: &'a Hub,
    id: u64,
    fmt: Formatter,
    status: Status,
    th: &'static Theme,
    width: usize,
    height: usize,

    // Scrollback keeps only the (shared) events; lines are rendered on demand.
    events: VecDeque<Arc<Event>>,
    scroll: usize, // lines scrolled up from the bottom
    unseen: usize, // lines that arrived while scrolled up

    input: Vec<char>,
    cursor: usize,
    hist: Vec<String>,
    hist_pos: usize,
    draft: String,

    last: HashMap<usize, String>, // last content drawn on fixed rows
    cur_col: usize,
    rang: Option<Instant>,
    out: String,
}

/// Drives a full-screen session until the user quits, the hub drops the
/// client, or the connection closes. Returns a goodbye line to print after
/// leaving the alternate screen.
pub async fn run(io: &mut Io, hub: &Hub, c: &mut Client, width: usize, height: usize) -> String {
    let status = hub.status(c.id);
    let th = theme_by_name(status.theme);
    let mut t = Term {
        hub,
        id: c.id,
        fmt: Formatter { styled: true, self_nick: status.nick.clone(), loc: status.loc, theme: th },
        status,
        th,
        width,
        height,
        events: VecDeque::new(),
        scroll: 0,
        unseen: 0,
        input: Vec::new(),
        cursor: 0,
        hist: Vec::new(),
        hist_pos: 0,
        draft: String::new(),
        last: HashMap::new(),
        cur_col: 0,
        rang: None,
        out: String::new(),
    };
    let mut dec = Decoder::default();
    // alt screen, bracketed paste, clear
    t.out.push_str("\x1b[?1049h\x1b[?2004h\x1b[H\x1b[2J");
    t.full_redraw();
    let mut alive = t.flush(io).await;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(5), Duration::from_secs(5));
    let msg = loop {
        if !alive {
            break String::new();
        }
        tokio::select! {
            ev = c.rx.recv() => {
                let Some(ev) = ev else { break goodbye(c) };
                // Plain chat lines don't change the fixed rows, so only refresh
                // status (which takes the hub lock) for other events.
                let mut stale = t.add_event(ev.clone());
                let mut ring = rings(&ev, &t.status.nick);
                for _ in 0..512 {
                    match c.rx.try_recv() {
                        Ok(ev) => {
                            stale |= t.add_event(ev.clone());
                            ring |= rings(&ev, &t.status.nick);
                        }
                        Err(_) => break,
                    }
                }
                if stale {
                    t.refresh_status();
                }
                if ring && t.status.bell && t.rang.is_none_or(|r| r.elapsed() > Duration::from_secs(2)) {
                    t.out.push('\x07');
                    t.rang = Some(Instant::now());
                }
                alive = t.flush(io).await;
            }
            inp = io.input.recv() => match inp {
                Some(Input::Data(d)) => {
                    let mut quit = false;
                    for k in dec.feed(&d) {
                        if t.handle_key(k) {
                            quit = true;
                            break;
                        }
                    }
                    if quit {
                        break "再见！Bye!".to_string();
                    }
                    alive = t.flush(io).await;
                }
                Some(Input::Resize(w, h)) => {
                    t.width = w;
                    t.height = h;
                    t.scroll = 0;
                    t.unseen = 0;
                    t.full_redraw();
                    alive = t.flush(io).await;
                }
                Some(Input::Eof) | None => break String::new(),
            },
            _ = tick.tick() => {
                t.refresh_status();
                alive = t.flush(io).await;
            }
            _ = c.done.changed() => break goodbye(c),
        }
    };
    t.out.push_str("\x1b[0m\x1b[r\x1b[?2004l\x1b[?1049l");
    t.out.push_str(RESET_DEFAULTS);
    t.flush(io).await;
    msg
}

fn goodbye(c: &Client) -> String {
    match done_reason(c) {
        Some(r) if !r.is_empty() => format!("已断开 Disconnected: {r}"),
        _ => "再见！Bye!".into(),
    }
}

fn unread_label(n: u64) -> String {
    if n > 99 { "99+".into() } else { n.to_string() }
}

impl Term<'_> {
    async fn flush(&mut self, io: &mut Io) -> bool {
        if self.out.is_empty() {
            return true;
        }
        let ok = io.write(self.out.as_bytes()).await;
        if self.out.capacity() > 16 << 10 {
            self.out = String::new(); // a full redraw grew it; don't keep it per session
        } else {
            self.out.clear();
        }
        ok
    }

    // --- geometry ---

    fn too_small(&self) -> bool {
        self.width < MIN_WIDTH || self.height < MIN_HEIGHT
    }
    fn chat_top(&self) -> usize {
        3
    }
    fn chat_bot(&self) -> usize {
        self.height.saturating_sub(3)
    }
    fn body_h(&self) -> usize {
        (self.chat_bot() + 1).saturating_sub(self.chat_top()).max(1)
    }
    fn wrap_w(&self) -> usize {
        self.width.saturating_sub(1).max(10) // never touch the last column
    }
    fn move_to(&mut self, row: usize, col: usize) {
        let _ = write!(self.out, "\x1b[{row};{col}H");
    }

    // --- events & scrollback ---

    fn render(&self, ev: &Event) -> Vec<String> {
        let w = self.wrap_w();
        let lines = self.fmt.lines(ev);
        if lines.iter().all(|l| width(l) <= w) {
            return lines;
        }
        const INDENT: &str = "        ";
        let mut out = Vec::new();
        for l in lines {
            if width(&l) <= w {
                out.push(l);
                continue;
            }
            // continuation lines are indented under the message text and keep
            // the style a line was cut in (a mention band, an error)
            let mut carry = String::new();
            for (i, mut wl) in wrap(&l, w - INDENT.len()).into_iter().enumerate() {
                if i > 0 {
                    wl = format!("{INDENT}{carry}{wl}");
                }
                carry = active_style(&wl, &self.th.base);
                out.push(truncate(&wl, w, ""));
            }
        }
        out
    }

    /// Appends and draws an event; reports whether it may have changed the
    /// status rows (joins, nick changes, room switches, ...).
    fn add_event(&mut self, ev: Arc<Event>) -> bool {
        if ev.kind == Kind::History {
            self.add_history(&ev.batch);
            return true;
        }
        let stale = !matches!(ev.kind, Kind::Chat | Kind::Action | Kind::DmIn | Kind::DmOut);
        self.events.push_back(ev.clone());
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        if self.too_small() {
            return stale;
        }
        let r = self.render(&ev);
        if self.scroll > 0 {
            // reader is looking at history: don't move the view
            self.scroll += r.len();
            self.unseen += r.len();
            self.draw_separator();
            return stale;
        }
        // Scroll the chat region: cursor on its last row, a newline per line;
        // the terminal shifts the region up by itself.
        self.move_to(self.chat_bot(), 1);
        self.out.push_str(&self.th.base); // the scrolled-in row takes the background
        for l in r {
            self.out.push_str("\n\r");
            self.out.push_str(&l);
            self.out.push_str(&self.th.base);
            self.out.push_str("\x1b[K");
        }
        self.place_cursor();
        stale
    }

    /// A replayed room history: appended and drawn with one redraw.
    fn add_history(&mut self, batch: &[Arc<Event>]) {
        if batch.is_empty() {
            return;
        }
        let skip = batch.len().saturating_sub(MAX_EVENTS);
        self.events.extend(batch[skip..].iter().cloned());
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        if self.too_small() {
            return;
        }
        self.scroll = 0;
        self.unseen = 0;
        self.draw_chat();
        self.draw_separator();
        self.place_cursor();
    }

    /// The lines visible with the current scroll offset, walking back from
    /// the newest event. Clamps the scroll offset at the top of history.
    fn window(&mut self) -> Vec<String> {
        let h = self.body_h();
        let need = self.scroll + h;
        let mut rev: Vec<String> = Vec::new(); // newest line first
        for ev in self.events.iter().rev() {
            if rev.len() >= need {
                break;
            }
            let r = self.render(ev);
            rev.extend(r.into_iter().rev());
        }
        if rev.len() < need {
            self.scroll = rev.len().saturating_sub(h);
            if self.scroll == 0 {
                self.unseen = 0;
            }
        }
        let end = rev.len().min(self.scroll + h);
        (self.scroll..end).rev().map(|i| rev[i].clone()).collect()
    }

    fn clear_chat(&mut self) {
        self.events.clear();
        self.scroll = 0;
        self.unseen = 0;
        self.draw_chat();
        self.draw_separator();
        self.place_cursor();
    }

    fn scroll_by(&mut self, n: isize) {
        let s = (self.scroll as isize + n).max(0) as usize;
        if s == self.scroll {
            return;
        }
        self.scroll = s;
        if s == 0 {
            self.unseen = 0;
        }
        self.draw_chat();
        self.draw_separator();
        self.place_cursor();
    }

    // --- drawing ---

    fn full_redraw(&mut self) {
        self.last.clear();
        // Base before erasing: the terminal fills cleared cells with the
        // current background, which paints the whole screen.
        self.out.push_str("\x1b[r");
        self.out.push_str(&self.th.defaults);
        self.out.push_str(&self.th.base);
        self.out.push_str("\x1b[H\x1b[2J");
        if self.too_small() {
            self.out.push_str(&bi("窗口太小", "Window too small"));
            return;
        }
        self.out.push_str("\x1b]2;VIVANT\x07");
        let _ = write!(self.out, "\x1b[{};{}r", self.chat_top(), self.chat_bot());
        let (h, r) = (self.header_line(), self.rooms_line());
        self.draw_fixed(1, h);
        self.draw_fixed(2, r);
        self.draw_chat();
        self.draw_separator();
        let f = self.footer_line();
        self.draw_fixed(self.height, f);
        self.draw_input();
    }

    fn draw_chat(&mut self) {
        if self.too_small() {
            return;
        }
        let h = self.body_h();
        let vis = self.window();
        let pad = h - vis.len();
        for i in 0..h {
            self.move_to(self.chat_top() + i, 1);
            if i >= pad {
                self.out.push_str(&vis[i - pad]);
                self.out.push_str(&self.th.base);
            }
            self.out.push_str("\x1b[K");
        }
    }

    /// Redraws a fixed row if its content changed.
    fn draw_fixed(&mut self, row: usize, content: String) {
        if self.too_small() || self.last.get(&row) == Some(&content) {
            return;
        }
        self.out.push_str("\x1b7"); // save cursor
        self.move_to(row, 1);
        self.out.push_str(&content);
        self.out.push_str(&self.th.base);
        self.out.push_str("\x1b[K\x1b8");
        self.last.insert(row, content);
    }

    fn refresh_status(&mut self) {
        self.status = self.hub.status(self.id);
        self.fmt.self_nick = self.status.nick.clone();
        self.fmt.loc = self.status.loc;
        if self.status.theme != self.th.name {
            self.th = theme_by_name(self.status.theme);
            self.fmt.theme = self.th;
            self.full_redraw();
            return;
        }
        let (h, r) = (self.header_line(), self.rooms_line());
        self.draw_fixed(1, h);
        self.draw_fixed(2, r);
        self.draw_separator();
    }

    fn header_line(&self) -> String {
        let s = &self.status;
        let now = s.loc.at(Utc::now());
        let left = " VIVANT ";
        let mut right =
            format!(" {} · 在线 Online {} · 运行 Up {} · {} {} ", s.nick, s.online, uptime(s.uptime), now.dt.format("%H:%M"), now.abbrev);
        if width(left) + width(&right) > self.width {
            right = format!(" {} · {} online ", s.nick, s.online);
        }
        let gap = self.width.saturating_sub(width(left) + width(&right));
        truncate(&(self.th.header.r(left) + &self.th.header_dim.r(&(" ".repeat(gap) + &right))), self.width, "")
    }

    fn rooms_line(&self) -> String {
        let mut b = self.th.dim.r("房间 ROOMS ");
        for r in &self.status.rooms {
            let item = format!("{} {}", r.label, r.users);
            if r.current {
                b.push_str(&self.th.cur_room.r(&format!("▸{item}")));
            } else if r.mentioned {
                b.push_str(&format!(" {item}{}", self.th.error.r(&format!(" @{}", unread_label(r.unread)))));
            } else if r.unread > 0 {
                b.push_str(&format!(" {item}{}", self.th.notice.r(&format!(" +{}", unread_label(r.unread)))));
            } else {
                b.push_str(&format!(" {item}"));
            }
            b.push_str("  ");
        }
        truncate(&b, self.width, "…")
    }

    fn draw_separator(&mut self) {
        let s = &self.status;
        let mut txt = format!("── {} · {} 人在线 here", s.room_label, s.room_users);
        if !s.topic.is_empty() {
            txt.push_str(" · ");
            txt.push_str(&s.topic);
        }
        txt.push(' ');
        let mut line = self.th.dim.r(&txt);
        if s.muted {
            line.push_str(&self.th.error.r("[MUTED] "));
        }
        if s.admin {
            line.push_str(&self.th.notice.r("[ADMIN] "));
        }
        if self.scroll > 0 {
            let more = if self.unseen > 0 {
                format!("↓ {} 条新消息 new", self.unseen)
            } else {
                format!("↑ 已上翻 scrolled {}", self.scroll)
            };
            line.push_str(&self.th.notice.r(&format!("{more} · End 回到底部 bottom ")));
        }
        let pad = self.width.saturating_sub(width(&line));
        if pad > 0 {
            line.push_str(&self.th.dim.r(&"─".repeat(pad)));
        }
        let row = self.height.saturating_sub(2);
        let l = truncate(&line, self.width, "");
        self.draw_fixed(row, l);
    }

    fn footer_line(&self) -> String {
        let st = self.th.ok.r("● 已连接 CONNECTED");
        let options = [
            "Enter 发送 send · ↑↓ 历史 history · PgUp/PgDn 滚动 scroll · Tab 补全 complete · Esc 清空 clear · Ctrl+C 退出 quit",
            "Enter 发送 send · ↑↓ 历史 history · PgUp/PgDn 滚动 scroll · Ctrl+C 退出 quit",
            "Enter 发送 send · PgUp/PgDn 滚动 · Ctrl+C 退出 quit",
        ];
        let mut keys = options[0];
        for k in options {
            keys = k;
            if self.width as isize - width(keys) as isize - width(&st) as isize > 1 {
                break;
            }
        }
        let gap = (self.width as isize - width(keys) as isize - width(&st) as isize - 1).max(1) as usize;
        truncate(&format!("{}{}{st}", self.th.dim.r(keys), " ".repeat(gap)), self.width, "")
    }

    /// The visible part of the input and the cursor column.
    fn input_view(&self) -> (String, usize) {
        let prompt = "> ";
        let avail = self.width.saturating_sub(width(prompt) + 1).max(4);
        if self.input.is_empty() {
            let ph = truncate("说点什么… Type a message…  /help 帮助 help", avail, "");
            return (self.th.prompt.r(prompt) + &self.th.dim.r(&ph), width(prompt) + 1);
        }
        let w_of = |s: &[char]| s.iter().map(|c| super::text::char_width(*c)).sum::<usize>();
        // choose a start so the cursor stays visible
        let mut start = 0;
        while start < self.cursor && w_of(&self.input[start..self.cursor]) > avail - 1 {
            start += 1;
        }
        let mut b = String::new();
        let mut w = 0;
        for &r in &self.input[start..] {
            let rw = super::text::char_width(r).max(1);
            if w + rw > avail {
                break;
            }
            b.push(r);
            w += rw;
        }
        let col = width(prompt) + w_of(&self.input[start..self.cursor]) + 1;
        (self.th.prompt.r(prompt) + &b, col)
    }

    fn draw_input(&mut self) {
        if self.too_small() {
            return;
        }
        let (s, col) = self.input_view();
        self.cur_col = col;
        self.move_to(self.height - 1, 1);
        self.out.push_str(&self.th.base);
        self.out.push_str(&s);
        self.out.push_str(&self.th.base);
        self.out.push_str("\x1b[K");
        self.place_cursor();
    }

    fn place_cursor(&mut self) {
        if self.too_small() {
            return;
        }
        if self.cur_col == 0 {
            self.cur_col = self.input_view().1;
        }
        self.move_to(self.height - 1, self.cur_col.min(self.width));
    }

    // --- input ---

    /// Returns true to quit.
    fn handle_key(&mut self, k: Key) -> bool {
        match k {
            Key::CtrlC => return true,
            Key::CtrlD => {
                if self.input.is_empty() {
                    return true;
                }
                self.delete_at(self.cursor);
            }
            Key::Enter => return self.submit(),
            Key::Rune(r) => self.insert(&r.to_string()),
            // newlines in a paste become spaces: pasting never sends by accident
            Key::Paste(s) => self.insert(&s.replace("\r\n", " ").replace(['\n', '\r', '\t'], " ")),
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.delete_at(self.cursor);
                }
            }
            Key::Delete => self.delete_at(self.cursor),
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.input.len()),
            Key::Home => {
                if self.input.is_empty() {
                    self.scroll_by((MAX_EVENTS * 8) as isize); // clamped to the top
                    return false;
                }
                self.cursor = 0;
            }
            Key::End => {
                if self.input.is_empty() {
                    self.scroll_by(-(self.scroll as isize));
                    return false;
                }
                self.cursor = self.input.len();
            }
            Key::Up => self.history_move(-1),
            Key::Down => self.history_move(1),
            Key::PgUp => {
                self.scroll_by(self.body_h().saturating_sub(2).max(1) as isize);
                return false;
            }
            Key::PgDn => {
                self.scroll_by(-(self.body_h().saturating_sub(2).max(1) as isize));
                return false;
            }
            Key::Esc => {
                self.input.clear();
                self.cursor = 0;
                self.hist_pos = self.hist.len();
                self.scroll_by(-(self.scroll as isize));
            }
            Key::CtrlU => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
            }
            Key::CtrlK => self.input.truncate(self.cursor),
            Key::CtrlW => {
                let mut i = self.cursor;
                while i > 0 && self.input[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && self.input[i - 1] != ' ' {
                    i -= 1;
                }
                self.input.drain(i..self.cursor);
                self.cursor = i;
            }
            Key::CtrlL => {
                self.full_redraw();
                return false;
            }
            Key::Tab => self.complete(),
        }
        self.draw_input();
        false
    }

    fn insert(&mut self, s: &str) {
        let limit = self.hub.config().max_msg_runes + 64;
        let room = limit.saturating_sub(self.input.len());
        let rs: Vec<char> = s.chars().filter(|&r| printable(r)).take(room).collect();
        let n = rs.len();
        self.input.splice(self.cursor..self.cursor, rs);
        self.cursor += n;
    }

    fn delete_at(&mut self, i: usize) {
        if i < self.input.len() {
            self.input.remove(i);
        }
    }

    fn submit(&mut self) -> bool {
        let line: String = self.input.drain(..).collect();
        self.cursor = 0;
        if line.trim().is_empty() {
            self.draw_input();
            return false;
        }
        if self.hist.last() != Some(&line) {
            self.hist.push(line.clone());
            if self.hist.len() > MAX_INPUT_HIST {
                self.hist.remove(0);
            }
        }
        self.hist_pos = self.hist.len();
        self.draft.clear();
        if self.scroll > 0 {
            self.scroll_by(-(self.scroll as isize));
        }
        let res = self.hub.handle(self.id, &line);
        if res.quit {
            return true;
        }
        if res.clear {
            self.clear_chat();
        }
        self.refresh_status();
        self.draw_input();
        false
    }

    fn history_move(&mut self, d: isize) {
        if self.hist.is_empty() {
            return;
        }
        if self.hist_pos == self.hist.len() {
            self.draft = self.input.iter().collect();
        }
        self.hist_pos = (self.hist_pos as isize + d).clamp(0, self.hist.len() as isize) as usize;
        let v = if self.hist_pos < self.hist.len() { self.hist[self.hist_pos].clone() } else { self.draft.clone() };
        self.input = v.chars().collect();
        self.cursor = self.input.len();
    }

    /// Command and nickname completion on the word before the cursor.
    fn complete(&mut self) {
        let before: String = self.input[..self.cursor].iter().collect();
        let i = before.rfind(' ').map_or(0, |p| p + 1);
        let word = &before[i..];
        if word.is_empty() {
            return;
        }
        let mut cands: Vec<String> = if i == 0 && word.starts_with('/') {
            let lw = word.to_lowercase();
            COMPLETIONS.iter().filter(|c| c.starts_with(&lw)).map(|c| c.to_string()).collect()
        } else {
            let w = word.trim_start_matches('@').to_lowercase();
            self.hub.nicks().into_iter().filter(|n| n.to_lowercase().starts_with(&w)).collect()
        };
        if cands.is_empty() {
            return;
        }
        cands.sort();
        let mut pick = cands[0].clone();
        if cands.len() > 1 {
            pick = common_prefix(&cands);
            if pick.chars().count() <= word.chars().count() {
                let ev = Event::new(Kind::Info, cands.join("  "), "");
                self.add_event(Arc::new(ev));
                return;
            }
        } else if i == 0 && !pick.starts_with('/') {
            pick.push_str(": ");
        } else if !pick.ends_with(' ') {
            pick.push(' ');
        }
        let rest: Vec<char> = self.input[self.cursor..].to_vec();
        let head = format!("{}{pick}", &before[..i]);
        self.cursor = head.chars().count();
        self.input = head.chars().chain(rest).collect();
    }
}

fn common_prefix(s: &[String]) -> String {
    let mut p: Vec<char> = s[0].chars().collect();
    for x in &s[1..] {
        let r: Vec<char> = x.chars().collect();
        let mut n = 0;
        while n < p.len() && n < r.len() && p[n].to_lowercase().eq(r[n].to_lowercase()) {
            n += 1;
        }
        p.truncate(n);
    }
    p.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::super::testio;
    use super::*;
    use crate::chat::Config;

    async fn wait_for(cap: &testio::Capture, sub: &str) {
        for _ in 0..200 {
            if cap.text().contains(sub) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never saw {sub:?} in output:\n{}", cap.text().replace('\x1b', "\\e"));
    }

    #[tokio::test]
    async fn sends_and_renders() {
        let hub = Hub::new(Config { msg_burst: 100.0, msg_per_second: 100.0, ..Default::default() });
        let mut other = hub.connect("b", false).unwrap();
        let mut me = hub.connect("a", false).unwrap();
        let (mut io, tx, cap) = testio::io();
        let h = hub.clone();
        let task = tokio::spawn(async move { run(&mut io, &h, &mut me, 80, 24).await });
        tx.send(Input::Data("你好 world\r".as_bytes().to_vec())).await.unwrap();
        wait_for(&cap, "你好 world").await;
        // the other side got it as a chat line
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut got = false;
        while let Ok(ev) = other.rx.try_recv() {
            got |= ev.kind == Kind::Chat && ev.text == "你好 world";
        }
        assert!(got);
        hub.handle(other.id, "hello from b");
        wait_for(&cap, "hello from b").await;
        assert!(cap.text().contains("\x1b[?1049h") && cap.text().contains("VIVANT"));
        tx.send(Input::Data(b"/quit\r".to_vec())).await.unwrap();
        assert_eq!(task.await.unwrap(), "再见！Bye!");
        assert!(cap.text().ends_with(RESET_DEFAULTS));
    }

    #[tokio::test]
    async fn rings_on_dm_and_history_batch() {
        let hub = Hub::new(Config { msg_burst: 100.0, msg_per_second: 100.0, ..Default::default() });
        let other = hub.connect("b", false).unwrap();
        for i in 0..30 {
            hub.handle(other.id, &format!("old {i}"));
        }
        let mut me = hub.connect("a", false).unwrap();
        let my_nick = hub.nick(me.id);
        let (mut io, tx, cap) = testio::io();
        let h = hub.clone();
        let task = tokio::spawn(async move { run(&mut io, &h, &mut me, 80, 24).await });
        wait_for(&cap, "old 29").await;
        hub.handle(other.id, &format!("/msg {my_nick} ping"));
        wait_for(&cap, "ping").await;
        assert!(cap.text().contains('\x07'), "no bell on DM");
        drop(tx);
        assert_eq!(task.await.unwrap(), "");
    }

    #[tokio::test]
    async fn kicked_session_ends_with_reason() {
        let hub = Hub::new(Config::default());
        let adm = hub.connect("adm", true).unwrap();
        let mut me = hub.connect("a", false).unwrap();
        let nick = hub.nick(me.id);
        let (mut io, _tx, _cap) = testio::io();
        let h = hub.clone();
        let task = tokio::spawn(async move { run(&mut io, &h, &mut me, 80, 24).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        hub.handle(adm.id, &format!("/kick {nick} bye"));
        let msg = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert!(msg.contains("已断开 Disconnected") && msg.contains("bye"), "{msg}");
    }

    #[test]
    fn prefix() {
        assert_eq!(common_prefix(&["alice".into(), "Alfred".into()]), "al");
    }
}
