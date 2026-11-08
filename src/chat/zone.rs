//! Display time zones for /tz: IANA names (Asia/Shanghai), "utc"/"local",
//! and fixed offsets like +8, -5, +05:30.

use chrono::{DateTime, FixedOffset, Local, Offset, Utc};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Zone {
    Utc,
    #[default]
    Local,
    /// seconds east of UTC
    Fixed(i32),
    Named(chrono_tz::Tz),
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Zone::Utc => f.write_str("UTC"),
            Zone::Local => f.write_str("Local"),
            Zone::Fixed(off) => {
                let sign = if *off < 0 { '-' } else { '+' };
                let a = off.unsigned_abs();
                let (h, m) = (a / 3600, a % 3600 / 60);
                if m > 0 { write!(f, "UTC{sign}{h}:{m:02}") } else { write!(f, "UTC{sign}{h}") }
            }
            Zone::Named(tz) => f.write_str(tz.name()),
        }
    }
}

/// A point in time converted to a zone: enough to format and compare days.
pub struct ZonedTime {
    pub dt: DateTime<FixedOffset>,
    pub abbrev: String,
}

impl Zone {
    pub fn at(&self, t: DateTime<Utc>) -> ZonedTime {
        match self {
            Zone::Utc => ZonedTime { dt: t.fixed_offset(), abbrev: "UTC".into() },
            Zone::Local => {
                let l = t.with_timezone(&Local);
                ZonedTime { abbrev: offset_abbrev(l.offset().fix().local_minus_utc()), dt: l.fixed_offset() }
            }
            Zone::Fixed(off) => {
                let fo = FixedOffset::east_opt(*off).unwrap_or(FixedOffset::east_opt(0).unwrap());
                ZonedTime { dt: t.with_timezone(&fo), abbrev: self.to_string() }
            }
            Zone::Named(tz) => {
                let l = t.with_timezone(tz);
                let abbrev = l.format("%Z").to_string();
                ZonedTime { dt: l.with_timezone(&l.offset().fix()), abbrev }
            }
        }
    }
}

fn offset_abbrev(off: i32) -> String {
    if off == 0 {
        return "UTC".into();
    }
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.unsigned_abs();
    format!("{sign}{:02}{:02}", a / 3600, a % 3600 / 60)
}

pub fn parse_zone(s: &str) -> Result<Zone, &'static str> {
    let s = s.trim();
    match s.to_lowercase().as_str() {
        "utc" | "gmt" | "z" => return Ok(Zone::Utc),
        "local" | "server" => return Ok(Zone::Local),
        _ => {}
    }
    if let Some(z) = parse_offset(s)? {
        return Ok(z);
    }
    if s.contains("..") || s.len() > 40 || s.is_empty() {
        return Err("bad zone");
    }
    chrono_tz::Tz::from_str_insensitive(s).map(Zone::Named).map_err(|_| "unknown zone")
}

/// `(?i)^(?:UTC|GMT)?([+-]?)(\d{1,2})(?::?(\d{2}))?$`
fn parse_offset(s: &str) -> Result<Option<Zone>, &'static str> {
    let mut rest = s;
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("utc") || lower.starts_with("gmt") {
        rest = &s[3..];
    }
    let (neg, rest) = match rest.as_bytes().first() {
        Some(b'+') => (false, &rest[1..]),
        Some(b'-') => (true, &rest[1..]),
        _ => (false, rest),
    };
    let digits: Vec<u8> = rest.bytes().collect();
    let hd = digits.iter().take_while(|b| b.is_ascii_digit()).count();
    if hd == 0 {
        return Ok(None);
    }
    let (h, m) = match (hd, &digits[hd.min(digits.len())..]) {
        (1 | 2, []) => (&rest[..hd], None),
        (1 | 2, [b':', a, b]) if a.is_ascii_digit() && b.is_ascii_digit() => (&rest[..hd], Some(&rest[hd + 1..])),
        (3, []) => (&rest[..1], Some(&rest[1..])),
        (4, []) => (&rest[..2], Some(&rest[2..])),
        _ => return Ok(None),
    };
    let hours: i32 = h.parse().map_err(|_| "bad offset")?;
    let mins: i32 = m.map(|m| m.parse().unwrap_or(99)).unwrap_or(0);
    if hours > 14 || mins > 59 {
        return Err("offset out of range");
    }
    let mut off = hours * 3600 + mins * 60;
    if neg {
        off = -off;
    }
    Ok(Some(Zone::Fixed(off)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse_zone("utc").unwrap(), Zone::Utc);
        assert_eq!(parse_zone("+8").unwrap(), Zone::Fixed(8 * 3600));
        assert_eq!(parse_zone("-5").unwrap(), Zone::Fixed(-5 * 3600));
        assert_eq!(parse_zone("UTC+05:30").unwrap(), Zone::Fixed(5 * 3600 + 1800));
        assert_eq!(parse_zone("gmt-0930").unwrap(), Zone::Fixed(-(9 * 3600 + 1800)));
        assert_eq!(parse_zone("8").unwrap(), Zone::Fixed(8 * 3600));
        assert!(parse_zone("+15").is_err());
        assert_eq!(parse_zone("asia/shanghai").unwrap().to_string(), "Asia/Shanghai");
        assert_eq!(parse_zone("America/New_York").unwrap().to_string(), "America/New_York");
        assert!(parse_zone("../../etc/passwd").is_err());
        assert!(parse_zone("Mars/Olympus").is_err());
        assert_eq!(Zone::Fixed(8 * 3600).to_string(), "UTC+8");
        assert_eq!(Zone::Fixed(-(5 * 3600 + 1800)).to_string(), "UTC-5:30");
    }
}
