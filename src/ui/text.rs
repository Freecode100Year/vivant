//! Display-width helpers that understand the escape sequences the UI itself
//! emits (CSI "\x1b[...X" and OSC "\x1b]...\x07").

use unicode_width::UnicodeWidthChar;

/// Splits `s` into (is_escape, piece) tokens; an escape is one whole CSI or
/// OSC sequence, every other token is one character.
fn tokens(s: &str) -> impl Iterator<Item = (bool, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        if i >= b.len() {
            return None;
        }
        let start = i;
        if b[i] == 0x1b && i + 1 < b.len() && b[i + 1] == b'[' {
            i += 2;
            while i < b.len() && !(0x40..=0x7e).contains(&b[i]) {
                i += 1;
            }
            i = (i + 1).min(b.len());
            return Some((true, &s[start..i]));
        }
        if b[i] == 0x1b && i + 1 < b.len() && b[i + 1] == b']' {
            i += 2;
            while i < b.len() && b[i] != 0x07 {
                i += 1;
            }
            i = (i + 1).min(b.len());
            return Some((true, &s[start..i]));
        }
        let ch = s[i..].chars().next().unwrap();
        i += ch.len_utf8();
        Some((false, &s[start..i]))
    })
}

pub fn char_width(c: char) -> usize {
    if c == '\u{200d}' || c == '\u{200c}' {
        return 0;
    }
    c.width().unwrap_or(0)
}

/// Display width of `s` in terminal cells, ignoring escape sequences.
pub fn width(s: &str) -> usize {
    if s.is_ascii() && !s.contains('\x1b') {
        return s.bytes().filter(|b| *b >= 0x20 && *b != 0x7f).count();
    }
    tokens(s).filter(|(esc, _)| !esc).map(|(_, t)| t.chars().map(char_width).sum::<usize>()).sum()
}

/// Cuts `s` to at most `w` cells (escape sequences preserved), appending
/// `tail` when something was cut.
pub fn truncate(s: &str, w: usize, tail: &str) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let tw = width(tail);
    let room = w.saturating_sub(tw);
    let mut out = String::with_capacity(s.len());
    let mut used = 0;
    let mut cut = false;
    for (esc, t) in tokens(s) {
        if esc {
            out.push_str(t);
            continue;
        }
        if cut {
            continue;
        }
        let cw: usize = t.chars().map(char_width).sum();
        if used + cw > room {
            cut = true;
            out.push_str(tail);
            continue;
        }
        used += cw;
        out.push_str(t);
    }
    out
}

/// Wraps `s` into lines of at most `w` cells, breaking at spaces where
/// possible and hard-breaking long words. Escape sequences are kept in place
/// (the caller re-applies the active style on continuation lines).
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    // the last breakable space in `cur`: (byte offset, width before it)
    let mut last_space: Option<(usize, usize)> = None;
    for (esc, t) in tokens(s) {
        if esc {
            cur.push_str(t);
            continue;
        }
        let cw: usize = t.chars().map(char_width).sum();
        if t == " " {
            if cur_w + 1 > w {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
                last_space = None;
                continue; // the space at a break disappears
            }
            last_space = Some((cur.len(), cur_w));
            cur.push(' ');
            cur_w += 1;
            continue;
        }
        if cur_w + cw > w {
            match last_space {
                Some((at, before)) if before > 0 => {
                    let rest = cur.split_off(at + 1);
                    cur.truncate(at);
                    lines.push(std::mem::take(&mut cur));
                    cur = rest;
                    cur_w -= before + 1;
                }
                _ => {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
            }
            last_space = None;
        }
        cur.push_str(t);
        cur_w += cw;
    }
    lines.push(cur);
    lines
}

/// The SGR sequence still in effect at the end of `s`, or "" if `s` ends in
/// the base style. Theme styles are each one complete SGR, so the last one
/// seen is the whole state.
pub fn active_style(s: &str, base: &str) -> String {
    let mut last = "";
    for (esc, t) in tokens(s) {
        if esc && t.ends_with('m') && t.starts_with("\x1b[") {
            last = t;
        }
    }
    if last == base || last == "\x1b[m" || last == "\x1b[0m" { String::new() } else { last.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("你好"), 4);
        assert_eq!(width("\x1b[1;38;5;19mab\x1b[0m"), 2);
        assert_eq!(width("\x1b]11;#ffffff\x07x"), 1);
        assert_eq!(width("👍"), 2);
    }

    #[test]
    fn truncates_keeping_escapes() {
        assert_eq!(truncate("hello world", 5, ""), "hello");
        assert_eq!(truncate("hello world", 6, "…"), "hello…");
        assert_eq!(truncate("\x1b[1mhello\x1b[0m", 3, ""), "\x1b[1mhel\x1b[0m");
        assert_eq!(truncate("你好世界", 5, ""), "你好");
    }

    #[test]
    fn wraps() {
        assert_eq!(wrap("the quick brown fox", 10), vec!["the quick", "brown fox"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("你好世界你好", 5), vec!["你好", "世界", "你好"]);
        let w = wrap("\x1b[1maaa bbb\x1b[0m", 4);
        assert_eq!(w.len(), 2);
        assert!(w.iter().all(|l| width(l) <= 4));
    }

    #[test]
    fn active() {
        let base = "\x1b[0;38;5;16;48;5;194m";
        assert_eq!(active_style("x\x1b[1;38;5;88mred", base), "\x1b[1;38;5;88m");
        assert_eq!(active_style(&format!("x\x1b[1mred{base}"), base), "");
    }
}
