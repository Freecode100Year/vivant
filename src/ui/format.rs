//! Turns chat events into display lines, in both languages.

use super::style::{Paint, Theme, theme_by_name};
use super::text::width;
use crate::chat::sanitize::is_nick_char;
use crate::chat::{Event, Kind, THEME_GREEN, Zone, bi};
use chrono::{Datelike, Utc};

/// A server message whose two halves fit in this many columns is shown on
/// one line ("中文 · English"); longer ones put the English on the next line.
const JOIN_WIDTH: usize = 78;

#[derive(Clone)]
pub struct Formatter {
    pub styled: bool,
    /// the viewer's current nickname
    pub self_nick: String,
    pub loc: Zone,
    pub theme: &'static Theme,
}

impl Formatter {
    pub fn new(styled: bool) -> Formatter {
        Formatter { styled, self_nick: String::new(), loc: Zone::Local, theme: theme_by_name(THEME_GREEN) }
    }

    fn st(&self, p: &Paint, s: &str) -> String {
        if self.styled { p.r(s) } else { s.to_string() }
    }

    /// Renders one event to one or more unwrapped lines.
    pub fn lines(&self, ev: &Event) -> Vec<String> {
        let th = self.theme;
        if ev.history && self.styled {
            // replayed history: whole line dimmed
            let plain = Formatter { styled: false, ..self.clone() };
            return plain.lines(ev).into_iter().map(|l| th.dim.r(&l)).collect();
        }
        // Server messages carry both languages: join them, or move the English
        // to its own line when the pair is long.
        let mut text = ev.text.clone();
        let mut en_line = String::new();
        if !ev.en.is_empty() && ev.en != ev.text {
            if ev.kind == Kind::System || width(&ev.text) + 3 + width(&ev.en) <= JOIN_WIDTH {
                text = bi(&ev.text, &ev.en);
            } else {
                en_line = ev.en.clone();
            }
        }
        let t = self.loc.at(ev.time);
        let mut stamp = t.dt.format("%H:%M").to_string();
        // History is kept for a day: a time alone would be ambiguous.
        if ev.history {
            let now = self.loc.at(Utc::now());
            if t.dt.ordinal() != now.dt.ordinal() || t.dt.year() != now.dt.year() {
                stamp = t.dt.format("%m-%d %H:%M").to_string();
            }
        }
        let ts = self.st(&th.dim, &format!("[{stamp}] "));
        let me = self.self_nick.as_str();
        let nick = |n: &str| -> String {
            if !self.styled {
                n.to_string()
            } else if n == me {
                th.self_.r(n)
            } else {
                th.nick_color(n).r(n)
            }
        };
        if ev.kind == Kind::Chat || ev.kind == Kind::Action {
            if ev.from != me && mentions(&text, me) {
                // a solid band plus a marker: visible without telling colours apart
                text = if self.styled { th.mention.r(&format!("» {text} ")) } else { format!("» {text}") };
            } else if self.styled {
                text = highlight_tags(&text, &th.tag);
            }
        }
        let head = match ev.kind {
            Kind::Chat => format!("{ts}<{}> {text}", nick(&ev.from)),
            Kind::Action => format!("{ts}* {} {text}", nick(&ev.from)),
            Kind::System => format!("{ts}{}", self.st(&th.dim, &format!("-- {text}"))),
            Kind::DmIn => format!("{ts}{}{text}", self.st(&th.dm, &format!("[私信 DM ← {}] ", ev.from))),
            Kind::DmOut => format!("{ts}{}{text}", self.st(&th.dm, &format!("[私信 DM → {}] ", ev.to))),
            Kind::Info => format!("{ts}{}", self.st(&th.info, &format!("* {text}"))),
            Kind::Error => format!("{ts}{}", self.st(&th.error, &format!("! {text}"))),
            Kind::Mention => format!(
                "{ts}{} {text}{}",
                self.st(&th.mention, &format!("@ #{} <{}> ", ev.room, ev.from)),
                self.st(&th.dim, &format!("  · /join {}", ev.room))
            ),
            Kind::Notice => {
                let from = if ev.from.is_empty() { "系统 System".to_string() } else { format!("管理员 Admin {}", ev.from) };
                format!("{ts}{}", self.st(&th.notice, &format!("📢 [{from}] {text}")))
            }
            Kind::History => format!("{ts}{text}"),
        };
        let mut out = vec![head];
        let body = |l: &str, en: bool, out: &mut Vec<String>| {
            let s = if en {
                self.st(&th.dim, l)
            } else if ev.kind == Kind::Info || ev.kind == Kind::Error {
                self.st(&th.info, l)
            } else {
                l.to_string()
            };
            out.push(format!("        {s}"));
        };
        if !en_line.is_empty() {
            body(&en_line, true, &mut out);
        }
        for (i, l) in ev.lines.iter().enumerate() {
            body(l, false, &mut out);
            if let Some(e) = ev.lines_en.get(i)
                && e != l
            {
                body(e, true, &mut out);
            }
        }
        out
    }
}

fn is_word_byte(s: &[u8], i: isize) -> bool {
    if i < 0 || i as usize >= s.len() {
        return false;
    }
    let c = s[i as usize];
    c == b'_' || c == b'-' || c.is_ascii_digit() || c.is_ascii_lowercase()
}

/// Whether `text` mentions `nick` (case-insensitive, whole word for Latin
/// nicknames; substring for CJK, which has no word boundaries).
pub fn mentions(text: &str, nick: &str) -> bool {
    if nick.is_empty() {
        return false;
    }
    let lt = text.to_lowercase();
    let ln = nick.to_lowercase();
    let b = lt.as_bytes();
    let mut i = 0;
    while let Some(j) = lt[i..].find(&ln) {
        let start = i + j;
        let end = start + ln.len();
        if !is_word_byte(b, start as isize - 1) && !is_word_byte(b, end as isize) {
            return true;
        }
        i = start + lt[start..].chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

/// Styles every "@nick" in text, so a mention of someone stands out
/// (underlined, not just coloured). "@" inside a word (an e-mail address)
/// is left alone.
pub fn highlight_tags(text: &str, p: &Paint) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 32);
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'@' || (i > 0 && is_word_byte(&[bytes[i - 1].to_ascii_lowercase()], 0)) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut n = 0;
        for c in text[j..].chars() {
            if !is_nick_char(c) || n == 16 {
                break;
            }
            j += c.len_utf8();
            n += 1;
        }
        if n < 2 {
            // nicknames are at least 2 characters
            i = j;
            continue;
        }
        out.push_str(&text[last..i]);
        out.push_str(&p.r(&text[i..j]));
        last = j;
        i = j;
    }
    if last == 0 {
        return text.to_string();
    }
    out.push_str(&text[last..]);
    out
}

/// Whether an event should sound the terminal bell for the viewer: a private
/// message, a mention from another room, or a line here that mentions them.
/// Replayed history never rings.
pub fn rings(ev: &Event, me: &str) -> bool {
    if ev.history {
        return false;
    }
    match ev.kind {
        Kind::DmIn | Kind::Mention => true,
        Kind::Chat | Kind::Action => ev.from != me && mentions(&ev.text, me),
        _ => false,
    }
}

/// "4d 12h" / "3h 5m" / "42m".
pub fn uptime(d: std::time::Duration) -> String {
    let m = (d.as_secs() + 30) / 60;
    let (days, hours, mins) = (m / 1440, m / 60 % 24, m % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::THEME_LIGHT;

    #[test]
    fn mention_rules() {
        assert!(mentions("hi alice", "Alice"));
        assert!(mentions("@Alice: yo", "alice"));
        assert!(!mentions("malice", "alice"));
        assert!(!mentions("alice_2 here", "alice"));
        assert!(mentions("夜猫子你好", "夜猫子"));
        assert!(!mentions("anything", ""));
    }

    #[test]
    fn tags() {
        let p = Paint { on: "<".into(), off: ">".into(), fg: 16, bg: None };
        for (input, want) in [
            ("@Antigravity 你在做什么", "<@Antigravity> 你在做什么"),
            ("hi @徐徐 and @bob_2!", "hi <@徐徐> and <@bob_2>!"),
            ("mail a@b.com", "mail a@b.com"),
            ("@ x @y", "@ x @y"),
            ("no tags", "no tags"),
        ] {
            assert_eq!(highlight_tags(input, &p), want, "{input}");
        }
    }

    #[test]
    fn rendering() {
        let ev = Event { id: 1, time: Utc::now(), kind: Kind::Chat, from: "bob".into(), text: "hey @carol".into(), ..Default::default() };
        let light = Formatter { theme: theme_by_name(THEME_LIGHT), ..Formatter::new(true) };
        let line = &light.lines(&ev)[0];
        assert!(line.contains(&light.theme.tag.r("@carol")) && !line.contains("\x1b[m"), "{line:?}");
        let me = Formatter { self_nick: "carol".into(), ..Formatter::new(true) };
        assert!(me.lines(&ev)[0].contains(&me.theme.mention.r("» hey @carol ")));
        let plain = Formatter::new(false).lines(&ev);
        assert!(!plain[0].contains('\x1b') && plain[0].ends_with("<bob> hey @carol"));
    }

    #[test]
    fn bilingual_layout() {
        let short = Event::new(Kind::Info, "已进入", "Now in");
        assert!(Formatter::new(false).lines(&short)[0].ends_with("* 已进入 · Now in"));
        let long = Event::new(Kind::Info, "很长".repeat(20), "very long ".repeat(5));
        let l = Formatter::new(false).lines(&long);
        assert_eq!(l.len(), 2);
        assert!(l[1].trim_start().starts_with("very long"));
    }

    #[test]
    fn timezone_and_history_date() {
        let mut ev = Event::new(Kind::Chat, "x", "");
        ev.time = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:00Z").unwrap().with_timezone(&Utc);
        let f = Formatter { loc: Zone::Fixed(8 * 3600), ..Formatter::new(false) };
        assert!(f.lines(&ev)[0].starts_with("[11:04]"));
        ev.history = true;
        assert!(f.lines(&ev)[0].starts_with("[01-02 11:04]"));
    }

    #[test]
    fn bells() {
        let mut ev = Event::new(Kind::DmIn, "hi", "");
        assert!(rings(&ev, "me"));
        ev.history = true;
        assert!(!rings(&ev, "me"));
        let ev = Event { kind: Kind::Chat, from: "x".into(), text: "hey me".into(), ..Default::default() };
        assert!(rings(&ev, "me"));
        assert!(!rings(&Event { from: "me".into(), ..ev }, "me"));
    }
}
