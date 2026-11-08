//! Text hygiene for everything users can make others see: messages,
//! nicknames, topics, room names.

use unicode_general_category::{GeneralCategory as G, get_general_category};

/// Removes anything that could manipulate a terminal or spoof the display:
/// C0/C1 control characters (which kills every ANSI/OSC escape sequence,
/// because they all start with ESC 0x1b or CSI 0x9b), DEL, Unicode bidi
/// overrides/isolates, and invisible format characters (zero-width spaces,
/// word joiners, "tag" characters that can smuggle hidden text). Tabs become
/// spaces. The result is trimmed and capped at `max_runes` characters
/// (0 = no cap).
///
/// Kept on purpose: ZWJ/ZWNJ (emoji sequences, Persian, Indic scripts) and
/// tag characters directly inside a 🏴 flag sequence (England, Scotland,
/// Wales flags).
pub fn sanitize(s: &str, max_runes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(4 * max_runes.max(16)));
    let mut n = 0;
    let mut in_flag = false; // after U+1F3F4, inside its tag sequence
    for mut r in s.chars() {
        if r == '\t' {
            r = ' ';
        }
        if !printable(r) {
            continue;
        }
        if is_tag(r) {
            if !in_flag {
                continue;
            }
            in_flag = r != '\u{e007f}'; // CANCEL TAG ends the sequence
        } else {
            in_flag = r == '\u{1f3f4}';
        }
        if max_runes > 0 && n >= max_runes {
            break;
        }
        out.push(r);
        n += 1;
    }
    trim_owned(out)
}

fn trim_owned(s: String) -> String {
    let t = s.trim();
    if t.len() == s.len() { s } else { t.to_string() }
}

/// Reports whether `r` is safe to show in a terminal: no C0/C1 controls
/// (which covers every escape sequence), DEL, bidi controls or BOM.
pub fn printable(r: char) -> bool {
    let u = r as u32;
    if u < 0x20 || u == 0x7f || (0x80..=0x9f).contains(&u) {
        return false;
    }
    if is_bidi_control(r) || r == '\u{feff}' || r == '\u{fffd}' {
        return false;
    }
    if r == '\u{200c}' || r == '\u{200d}' || is_tag(r) {
        return true; // ZWNJ, ZWJ; tags are checked in context by sanitize
    }
    get_general_category(r) != G::Format // other invisible format characters
}

fn is_tag(r: char) -> bool {
    ('\u{e0020}'..='\u{e007f}').contains(&r)
}

fn is_bidi_control(r: char) -> bool {
    matches!(r, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')
}

pub fn is_letter(r: char) -> bool {
    matches!(get_general_category(r), G::UppercaseLetter | G::LowercaseLetter | G::TitlecaseLetter | G::ModifierLetter | G::OtherLetter)
}

pub fn is_digit(r: char) -> bool {
    get_general_category(r) == G::DecimalNumber
}

pub fn is_upper(r: char) -> bool {
    get_general_category(r) == G::UppercaseLetter
}

pub fn is_lower(r: char) -> bool {
    get_general_category(r) == G::LowercaseLetter
}

pub fn is_punct(r: char) -> bool {
    matches!(
        get_general_category(r),
        G::ConnectorPunctuation
            | G::DashPunctuation
            | G::OpenPunctuation
            | G::ClosePunctuation
            | G::InitialPunctuation
            | G::FinalPunctuation
            | G::OtherPunctuation
    )
}

/// Characters a nickname (and an @mention) is made of.
pub fn is_nick_char(r: char) -> bool {
    is_letter(r) || is_digit(r) || r == '_' || r == '-'
}

/// Reports whether a nickname is acceptable: 2–16 characters made of letters
/// (any script, so 中文昵称 works), digits, '_' and '-'.
pub fn valid_nick(n: &str) -> bool {
    let c = n.chars().count();
    (2..=16).contains(&c) && n.chars().all(is_nick_char)
}

const RESERVED: &[&str] = &["system", "admin", "administrator", "root", "you", "server", "moderator", "mod", "guest", "plain", "anonymous"];

pub fn reserved_nick(n: &str) -> bool {
    let l = n.to_lowercase();
    RESERVED.contains(&l.as_str())
}

/// 2–24 characters: lowercase letters, digits, '-' and '_'.
pub fn valid_room_name(n: &str) -> bool {
    let c = n.chars().count();
    (2..=24).contains(&c) && n.chars().all(|r| is_lower(r) || is_digit(r) || r == '_' || r == '-' || (is_letter(r) && !is_upper(r)))
}

/// Unicode whitespace as Go's unicode.IsSpace sees it.
pub fn is_space(r: char) -> bool {
    r.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes_and_invisibles() {
        let cases = [
            ("\x1b[31mred\x1b[0m", "[31mred[0m"),
            ("a\u{9b}31mb", "a31mb"),
            ("\x1b]52;c;aGk=\x07x", "]52;c;aGk=x"),
            ("x\u{200b}y\u{2060}z", "xyz"),
            ("evil\u{202e}txt", "eviltxt"),
            ("hidden\u{e0041}\u{e0042}", "hidden"),
            ("  tab\there  ", "tab here"),
            ("\u{feff}bom", "bom"),
            ("del\x7fete", "delete"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize(input, 500), want, "{input:?}");
        }
    }

    #[test]
    fn keeps_joiners_and_flags() {
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(sanitize(family, 500), family);
        let england = "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}";
        assert_eq!(sanitize(england, 500), england);
        assert_eq!(sanitize("می\u{200c}خواهم", 500), "می\u{200c}خواهم");
    }

    #[test]
    fn caps_length() {
        assert_eq!(sanitize("你好世界abc", 4), "你好世界");
    }

    #[test]
    fn nick_rules() {
        assert!(valid_nick("夜猫子"));
        assert!(valid_nick("alice_01"));
        assert!(!valid_nick("a"));
        assert!(!valid_nick("bad nick"));
        assert!(!valid_nick("x\u{200b}y"));
        assert!(!valid_nick("abcdefghijklmnopq"));
        assert!(reserved_nick("Admin"));
        assert!(valid_room_name("tech-2"));
        assert!(valid_room_name("技术"));
        assert!(!valid_room_name("Tech"));
        assert!(!valid_room_name("a"));
    }

    #[test]
    fn nothing_unprintable_survives() {
        // every code point, each between two letters
        let mut s = String::new();
        for u in 0..=0x10ffffu32 {
            if let Some(c) = char::from_u32(u) {
                s.push('a');
                s.push(c);
            }
        }
        let out = sanitize(&s, 0);
        let mut prev = ' ';
        for c in out.chars() {
            let u = c as u32;
            assert!(!(u < 0x20 || (0x7f..=0x9f).contains(&u)), "control {u:x}");
            assert!(!is_bidi_control(c) && c != '\u{feff}', "bidi/bom {u:x}");
            let cat = get_general_category(c);
            assert!(
                cat != G::Format || c == '\u{200c}' || c == '\u{200d}' || (is_tag(c) && (prev == '\u{1f3f4}' || is_tag(prev))),
                "format {u:x}"
            );
            prev = c;
        }
    }
}
