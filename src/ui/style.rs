//! Colours chosen for low vision and colour blindness:
//!
//! - Every theme paints its own background, so contrast is known rather than
//!   guessed. Every colour — body text, nicknames, timestamps, hints, errors —
//!   is at least 7:1 against it (WCAG AAA); body text is 15:1 or more.
//!   `theme_contrast` enforces this.
//! - Nickname and status hues come from the Okabe–Ito colour-blind-safe
//!   family. Nothing relies on telling red from green.
//! - Meaning never rides on colour alone: a line that mentions you is a solid
//!   band with a "»" marker, @nick tags are underlined, errors start with "!",
//!   the current room with "▸", unread counts with "+" and "@".
//!
//! Only the xterm 256-colour palette is used (no 24-bit colour), so Windows
//! conhost, macOS Terminal.app, tmux and phone SSH apps all show the same.

use crate::chat::{THEME_DARK, THEME_GREEN, THEME_LIGHT};
use std::sync::LazyLock;

/// A precomputed SGR prefix plus the sequence that returns to the theme's
/// base colours. Styling a string is a concatenation.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // fg/bg: checked by the contrast test
pub struct Paint {
    pub on: String,
    pub off: String,
    pub fg: u8,
    /// None = the theme background
    pub bg: Option<u8>,
}

impl Paint {
    /// Wraps `s` in the style, then returns to the theme's base colours.
    pub fn r(&self, s: &str) -> String {
        if self.on.is_empty() {
            return s.to_string();
        }
        let mut out = String::with_capacity(self.on.len() + s.len() + self.off.len());
        out.push_str(&self.on);
        out.push_str(s);
        out.push_str(&self.off);
        out
    }
}

/// A complete colour scheme. `base` sets the theme's foreground and
/// background; it is written wherever a plain reset would be, so the
/// background stays painted.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct Theme {
    pub name: &'static str,
    pub fg: u8,
    pub bg: u8,
    pub base: String,
    /// sets the terminal's default colours to the theme's (OSC 10/11)
    pub defaults: String,
    pub header: Paint,
    pub header_dim: Paint,
    pub cur_room: Paint,
    pub prompt: Paint,
    pub self_: Paint,
    pub mention: Paint,
    pub tag: Paint,
    pub dim: Paint,
    pub info: Paint,
    pub error: Paint,
    pub notice: Paint,
    pub dm: Paint,
    pub ok: Paint,
    pub nicks: Vec<Paint>,
}

/// Restores the terminal's own default colours (OSC 110/111).
pub const RESET_DEFAULTS: &str = "\x1b]110\x07\x1b]111\x07";

/// The standard RGB value of a 256-colour index (16–255; 0–15 are
/// user-configurable and deliberately unused by the themes).
pub fn xterm256(n: u8) -> (u8, u8, u8) {
    assert!(n >= 16, "system colour used in a theme");
    if n >= 232 {
        let v = 8 + (n - 232) * 10;
        return (v, v, v);
    }
    let n = n - 16;
    let lv = |i: u8| if i == 0 { 0 } else { 55 + i * 40 };
    (lv(n / 36), lv(n / 6 % 6), lv(n % 6))
}

pub fn hex256(n: u8) -> String {
    let (r, g, b) = xterm256(n);
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl Theme {
    fn new(name: &'static str, fg: u8, bg: u8) -> Theme {
        Theme {
            name,
            fg,
            bg,
            base: format!("\x1b[0;38;5;{fg};48;5;{bg}m"),
            // OSC 10/11 also set the terminal's own default colours, so cells
            // never painted (window padding, columns beyond the reported size,
            // rows ConPTY re-renders on Windows) show the theme too.
            defaults: format!("\x1b]10;{}\x07\x1b]11;{}\x07", hex256(fg), hex256(bg)),
            ..Default::default()
        }
    }

    /// A style: foreground fg, background bg (None = theme's), plus SGR
    /// attributes such as "1" (bold) or "1;4" (bold underline).
    fn p(&self, fg: u8, bg: Option<u8>, attr: &str) -> Paint {
        let mut on = String::from("\x1b[");
        if !attr.is_empty() {
            on.push_str(attr);
            on.push(';');
        }
        on.push_str(&format!("38;5;{fg}"));
        if let Some(bg) = bg {
            on.push_str(&format!(";48;5;{bg}"));
        }
        on.push('m');
        Paint { on, off: self.base.clone(), fg, bg }
    }

    fn set_nicks(&mut self, fgs: &[u8]) {
        self.nicks = fgs.iter().map(|&f| self.p(f, None, "")).collect();
    }

    /// The colour for a nickname: stable per name (FNV-1a of the lowercase
    /// name, as in the Go version, so people keep their colour).
    pub fn nick_color(&self, n: &str) -> &Paint {
        let mut h: u32 = 0x811c9dc5;
        for b in n.to_lowercase().bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        &self.nicks[(h % self.nicks.len() as u32) as usize]
    }
}

fn dark() -> Theme {
    let mut t = Theme::new(THEME_DARK, 231, 16);
    t.header = t.p(231, Some(18), "1");
    t.header_dim = t.p(255, Some(18), "");
    t.cur_room = t.p(117, None, "1");
    t.prompt = t.p(117, None, "1");
    t.self_ = t.p(231, None, "1");
    t.mention = t.p(16, Some(208), "1");
    t.tag = t.p(213, None, "1;4");
    t.dim = t.p(250, None, "");
    t.info = t.p(116, None, "");
    t.error = t.p(210, None, "1");
    t.notice = t.p(222, None, "");
    t.dm = t.p(183, None, "1");
    t.ok = t.p(117, None, "");
    //            orange sky  b-green yellow blue verm. purple teal  khaki lilac
    t.set_nicks(&[215, 117, 79, 227, 111, 209, 176, 152, 186, 147]);
    t
}

/// The styles shared by the two light backgrounds; the colours are dark
/// enough to pass on the (slightly darker) green as well.
fn light_common(mut t: Theme) -> Theme {
    t.header = t.p(231, Some(18), "1");
    t.header_dim = t.p(255, Some(18), "");
    t.cur_room = t.p(19, None, "1");
    t.prompt = t.p(19, None, "1");
    t.self_ = t.p(16, None, "1");
    t.mention = t.p(16, Some(208), "1");
    t.tag = t.p(90, None, "1;4");
    t.dim = t.p(238, None, "");
    t.info = t.p(22, None, "");
    t.error = t.p(88, None, "1");
    t.notice = t.p(52, None, "1");
    t.dm = t.p(55, None, "1");
    t.ok = t.p(19, None, "");
    // Only dark shades reach 7:1 on a light page.
    //           navy red green purple maroon blue violet wine indigo grape
    t.set_nicks(&[18, 88, 22, 90, 52, 20, 54, 89, 55, 56]);
    t
}

pub static THEMES: LazyLock<[Theme; 3]> = LazyLock::new(|| {
    [
        // Green: the "eye-care" pale green background (护眼绿), black text. Default.
        light_common(Theme::new(THEME_GREEN, 16, 194)),
        dark(),
        // Light: black on white.
        light_common(Theme::new(THEME_LIGHT, 16, 231)),
    ]
});

/// The named theme (the default, green, for unknown names).
pub fn theme_by_name(name: &str) -> &'static Theme {
    THEMES.iter().find(|t| t.name == name).unwrap_or(&THEMES[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(n: u8) -> f64 {
        let (r, g, b) = xterm256(n);
        let lin = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: u8, b: u8) -> f64 {
        let (mut la, mut lb) = (luminance(a), luminance(b));
        if la < lb {
            std::mem::swap(&mut la, &mut lb);
        }
        (la + 0.05) / (lb + 0.05)
    }

    #[test]
    fn theme_contrast() {
        for th in THEMES.iter() {
            assert!(contrast(th.fg, th.bg) >= 15.0, "{}: body text", th.name);
            let mut styles = vec![
                ("header", &th.header),
                ("header_dim", &th.header_dim),
                ("cur_room", &th.cur_room),
                ("prompt", &th.prompt),
                ("self", &th.self_),
                ("mention", &th.mention),
                ("tag", &th.tag),
                ("dim", &th.dim),
                ("info", &th.info),
                ("error", &th.error),
                ("notice", &th.notice),
                ("dm", &th.dm),
                ("ok", &th.ok),
            ];
            for n in &th.nicks {
                styles.push(("nick", n));
            }
            for (name, p) in styles {
                let bg = p.bg.unwrap_or(th.bg);
                let c = contrast(p.fg, bg);
                assert!(c >= 7.0, "{}/{name}: {c:.2}:1, want >= 7 (AAA)", th.name);
                assert_eq!(p.off, th.base, "{}/{name} does not return to the theme base", th.name);
            }
            // the mention band must stand out from the page by brightness too
            assert!(contrast(th.mention.bg.unwrap(), th.bg) >= 2.0, "{}: mention band too close", th.name);
        }
    }

    #[test]
    fn theme_defaults() {
        assert_eq!(theme_by_name(THEME_GREEN).defaults, "\x1b]10;#000000\x07\x1b]11;#d7ffd7\x07");
        assert_eq!(hex256(24), "#005f87");
        assert_eq!(hex256(250), "#bcbcbc");
        for input in ["black", "white", "green", "护眼", "dark", "light"] {
            let name = crate::chat::parse_theme(input).unwrap();
            assert_eq!(theme_by_name(name).name, name);
        }
    }
}
