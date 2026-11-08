//! Decodes the byte stream from an xterm-compatible terminal (including
//! Windows Terminal / conhost via OpenSSH) into keys. Input arrives in SSH
//! packets; the decoder keeps partial UTF-8 characters and escape sequences
//! across packets, and treats an ESC that ends a packet as the Esc key (a
//! lone Esc press arrives in a packet by itself).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Rune(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PgUp,
    PgDn,
    Tab,
    Esc,
    CtrlC,
    CtrlD,
    CtrlU,
    CtrlW,
    CtrlK,
    CtrlL,
    Paste(String),
}

const PASTE_END: &[u8] = b"\x1b[201~";
const PASTE_MAX: usize = 16 * 1024;

#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    paste: Option<Vec<u8>>,
    /// a paste that hit the size cap: discard until its end marker
    paste_overflow: bool,
    last_cr: bool,
}

enum Step {
    Key(Key, usize),
    Skip(usize),
    NeedMore,
}

impl Decoder {
    /// Feeds one packet and returns the complete keys in it.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Key> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        let mut i = 0;
        loop {
            if let Some(p) = self.paste.as_mut() {
                let rest = &self.buf[i..];
                match find(rest, PASTE_END) {
                    Some(end) => {
                        if !self.paste_overflow {
                            p.extend_from_slice(&rest[..end.min(PASTE_MAX - p.len().min(PASTE_MAX))]);
                            out.push(Key::Paste(String::from_utf8_lossy(p).into_owned()));
                        }
                        self.paste = None;
                        self.paste_overflow = false;
                        i += end + PASTE_END.len();
                        continue;
                    }
                    None => {
                        // keep a possible partial end marker for the next packet
                        let keep = PASTE_END.len() - 1;
                        let take = rest.len().saturating_sub(keep);
                        if !self.paste_overflow {
                            let room = PASTE_MAX - p.len();
                            p.extend_from_slice(&rest[..take.min(room)]);
                            if p.len() >= PASTE_MAX {
                                out.push(Key::Paste(String::from_utf8_lossy(p).into_owned()));
                                p.clear();
                                self.paste_overflow = true;
                            }
                        }
                        i += take;
                        break;
                    }
                }
            }
            if i >= self.buf.len() {
                break;
            }
            match self.step(i) {
                Step::Key(k, n) => {
                    i += n;
                    if k == Key::Paste(String::new()) {
                        self.paste = Some(Vec::new());
                    } else {
                        out.push(k);
                    }
                }
                Step::Skip(n) => i += n,
                Step::NeedMore => break,
            }
        }
        self.buf.drain(..i);
        if self.buf.len() > 64 && self.paste.is_none() {
            self.buf.clear(); // garbage that never completes
        }
        out
    }

    fn step(&mut self, i: usize) -> Step {
        let b = self.buf[i];
        let at_end = i + 1 == self.buf.len();
        let cr = self.last_cr;
        self.last_cr = b == b'\r';
        let k = match b {
            b'\r' => Key::Enter,
            b'\n' => {
                if cr {
                    return Step::Skip(1); // CRLF from some clients: one Enter
                }
                Key::Enter
            }
            0x7f | 0x08 => Key::Backspace,
            0x09 => Key::Tab,
            0x03 => Key::CtrlC,
            0x04 => Key::CtrlD,
            0x01 => Key::Home,
            0x05 => Key::End,
            0x02 => Key::Left,
            0x06 => Key::Right,
            0x10 => Key::Up,
            0x0e => Key::Down,
            0x15 => Key::CtrlU,
            0x17 => Key::CtrlW,
            0x0b => Key::CtrlK,
            0x0c => Key::CtrlL,
            0x1b => {
                if at_end {
                    return Step::Key(Key::Esc, 1);
                }
                return self.escape(i);
            }
            b if b < 0x20 => return Step::Skip(1),
            _ => return self.utf8(i),
        };
        Step::Key(k, 1)
    }

    fn escape(&self, i: usize) -> Step {
        let b = &self.buf[i..];
        match b[1] {
            b'[' => {
                for (n, &c) in b.iter().enumerate().skip(2).take(32) {
                    if (0x40..=0x7e).contains(&c) {
                        let params = std::str::from_utf8(&b[2..n]).unwrap_or("");
                        return match csi_key(params, c) {
                            Some(k) => Step::Key(k, n + 1),
                            None => Step::Skip(n + 1),
                        };
                    }
                }
                if b.len() < 34 { Step::NeedMore } else { Step::Skip(2) }
            }
            b'O' => match b.get(2) {
                Some(&c) => match csi_key("", c) {
                    Some(k) => Step::Key(k, 3),
                    None => Step::Skip(3),
                },
                None => Step::NeedMore,
            },
            0x1b => Step::Key(Key::Esc, 1),
            _ => Step::Skip(2), // Alt+key: ignored
        }
    }

    fn utf8(&self, i: usize) -> Step {
        let b = self.buf[i];
        let len = match b {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            0x20..=0x7f => 1,
            _ => return Step::Skip(1),
        };
        if self.buf.len() - i < len {
            return Step::NeedMore;
        }
        match std::str::from_utf8(&self.buf[i..i + len]) {
            Ok(s) => Step::Key(Key::Rune(s.chars().next().unwrap()), len),
            Err(_) => Step::Skip(1),
        }
    }
}

fn csi_key(params: &str, fin: u8) -> Option<Key> {
    Some(match fin {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::Tab, // shift+tab
        b'~' => match params.split(';').next().unwrap_or("") {
            "1" | "7" => Key::Home,
            "4" | "8" => Key::End,
            "3" => Key::Delete,
            "5" => Key::PgUp,
            "6" => Key::PgDn,
            "200" => Key::Paste(String::new()), // start of a bracketed paste
            _ => return None,
        },
        _ => return None,
    })
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(packets: &[&[u8]]) -> Vec<Key> {
        let mut d = Decoder::default();
        packets.iter().flat_map(|p| d.feed(p)).collect()
    }

    #[test]
    fn decodes() {
        assert_eq!(all(&[b"hi\r"]), vec![Key::Rune('h'), Key::Rune('i'), Key::Enter]);
        assert_eq!(all(&[b"a\r\nb"]), vec![Key::Rune('a'), Key::Enter, Key::Rune('b')]);
        assert_eq!(all(&[b"\x1b[A\x1b[B\x1bOC\x1b[5~\x1b[3~"]), vec![Key::Up, Key::Down, Key::Right, Key::PgUp, Key::Delete]);
        assert_eq!(all(&[b"\x1b"]), vec![Key::Esc]);
        assert_eq!(all(&[b"\x1b[", b"A"]), vec![Key::Up]);
        assert_eq!(all(&["你".as_bytes()]), vec![Key::Rune('你')]);
        let ni = "你".as_bytes();
        assert_eq!(all(&[&ni[..1], &ni[1..]]), vec![Key::Rune('你')]);
        assert_eq!(all(&[b"\x1b[200~line1\nline2\x1b[201~x"]), vec![Key::Paste("line1\nline2".into()), Key::Rune('x')]);
        assert_eq!(all(&[b"\x1b[200~ab", b"c\x1b[20", b"1~"]), vec![Key::Paste("abc".into())]);
        assert_eq!(
            all(&[b"\x03\x04\x15\x17\x0b\x0c\x7f\t"]),
            vec![Key::CtrlC, Key::CtrlD, Key::CtrlU, Key::CtrlW, Key::CtrlK, Key::CtrlL, Key::Backspace, Key::Tab]
        );
        assert_eq!(all(&[b"\x1bx"]), vec![]); // Alt+x ignored
        assert_eq!(all(&[b"\xff\xfeok"]), vec![Key::Rune('o'), Key::Rune('k')]);
    }

    #[test]
    fn huge_paste_is_capped() {
        let mut d = Decoder::default();
        let mut keys = d.feed(b"\x1b[200~");
        for _ in 0..100 {
            keys.extend(d.feed(&[b'a'; 1024]));
        }
        keys.extend(d.feed(b"\x1b[201~z"));
        let pasted: usize = keys.iter().map(|k| if let Key::Paste(s) = k { s.len() } else { 0 }).sum();
        assert!(pasted <= PASTE_MAX);
        assert_eq!(keys.last(), Some(&Key::Rune('z')));
    }
}
