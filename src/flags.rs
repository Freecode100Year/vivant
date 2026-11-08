//! Command-line flags with the same syntax as Go's `flag` package, which the
//! systemd unit, the installer and existing /etc/vivant.env files use:
//! `-addr :2222`, `-addr=:2222`, `--addr :2222`, booleans as `-v` or
//! `-tor-exits=false`, durations as `1h`, `90m`, `24h0m0s`. Parsing stops at
//! the first non-flag argument or `--`.

use std::collections::BTreeMap;
use std::time::Duration;

pub const HELP: &str = "help requested";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Dur(Duration),
}

struct Flag {
    value: Value,
    default: Value,
    usage: &'static str,
}

#[derive(Default)]
pub struct FlagSet {
    flags: BTreeMap<&'static str, Flag>,
}

impl FlagSet {
    pub fn new() -> FlagSet {
        FlagSet::default()
    }

    fn add(&mut self, name: &'static str, v: Value, usage: &'static str) {
        self.flags.insert(name, Flag { value: v.clone(), default: v, usage });
    }
    pub fn string(&mut self, name: &'static str, def: &str, usage: &'static str) {
        self.add(name, Value::Str(def.to_string()), usage)
    }
    pub fn int(&mut self, name: &'static str, def: i64, usage: &'static str) {
        self.add(name, Value::Int(def), usage)
    }
    pub fn float(&mut self, name: &'static str, def: f64, usage: &'static str) {
        self.add(name, Value::Float(def), usage)
    }
    pub fn bool(&mut self, name: &'static str, def: bool, usage: &'static str) {
        self.add(name, Value::Bool(def), usage)
    }
    pub fn duration(&mut self, name: &'static str, def: Duration, usage: &'static str) {
        self.add(name, Value::Dur(def), usage)
    }

    pub fn get_string(&self, n: &str) -> String {
        match &self.flags[n].value {
            Value::Str(s) => s.clone(),
            v => panic!("{n} is {v:?}"),
        }
    }
    pub fn get_int(&self, n: &str) -> i64 {
        match self.flags[n].value {
            Value::Int(i) => i,
            ref v => panic!("{n} is {v:?}"),
        }
    }
    pub fn get_float(&self, n: &str) -> f64 {
        match self.flags[n].value {
            Value::Float(f) => f,
            ref v => panic!("{n} is {v:?}"),
        }
    }
    pub fn get_bool(&self, n: &str) -> bool {
        match self.flags[n].value {
            Value::Bool(b) => b,
            ref v => panic!("{n} is {v:?}"),
        }
    }
    pub fn get_duration(&self, n: &str) -> Duration {
        match self.flags[n].value {
            Value::Dur(d) => d,
            ref v => panic!("{n} is {v:?}"),
        }
    }

    pub fn parse(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" {
                return Ok(args[i + 1..].to_vec());
            }
            if !a.starts_with('-') || a == "-" {
                return Ok(args[i..].to_vec());
            }
            let body = a.strip_prefix("--").unwrap_or(&a[1..]);
            if body.is_empty() || body.starts_with('-') || body.starts_with('=') {
                return Err(format!("bad flag syntax: {a}"));
            }
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            if name == "h" || name == "help" {
                return Err(HELP.into());
            }
            let Some(f) = self.flags.get_mut(name) else {
                return Err(format!("flag provided but not defined: -{name}"));
            };
            if let Value::Bool(_) = f.value {
                let v = match inline.as_deref() {
                    None => true,
                    Some(s) => parse_bool(s).ok_or_else(|| format!("invalid boolean value {s:?} for -{name}"))?,
                };
                f.value = Value::Bool(v);
                i += 1;
                continue;
            }
            let raw = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    args.get(i).cloned().ok_or_else(|| format!("flag needs an argument: -{name}"))?
                }
            };
            let bad = |what: &str| format!("invalid value {raw:?} for flag -{name}: {what}");
            f.value = match f.value {
                Value::Str(_) => Value::Str(raw.clone()),
                Value::Int(_) => Value::Int(raw.parse().map_err(|_| bad("parse error"))?),
                Value::Float(_) => Value::Float(raw.parse().map_err(|_| bad("parse error"))?),
                Value::Dur(_) => Value::Dur(parse_go_duration(&raw).ok_or_else(|| bad("invalid duration"))?),
                Value::Bool(_) => unreachable!(),
            };
            i += 1;
        }
        Ok(Vec::new())
    }

    /// The flag list in Go's `PrintDefaults` layout.
    pub fn defaults(&self) -> String {
        let mut out = String::new();
        for (name, f) in &self.flags {
            let (kind, def) = match &f.default {
                Value::Str(s) => ("string", if s.is_empty() { None } else { Some(format!("{s:?}")) }),
                Value::Int(i) => ("int", if *i == 0 { None } else { Some(i.to_string()) }),
                Value::Float(x) => ("float", if *x == 0.0 { None } else { Some(x.to_string()) }),
                Value::Bool(b) => ("", if *b { Some("true".to_string()) } else { None }),
                Value::Dur(d) => ("duration", if d.is_zero() { None } else { Some(go_duration(*d)) }),
            };
            out.push_str(&format!("  -{name}"));
            if !kind.is_empty() {
                out.push(' ');
                out.push_str(kind);
            }
            out.push_str(&format!("\n    \t{}", f.usage));
            if let Some(d) = def {
                out.push_str(&format!(" (default {d})"));
            }
            out.push('\n');
        }
        out
    }
}

/// Go's time.Duration.String() for whole seconds ("1h0m0s", "90s" → "1m30s").
pub fn go_duration(d: Duration) -> String {
    let total = d.as_secs();
    if total == 0 {
        let ms = d.as_millis();
        return if ms > 0 { format!("{ms}ms") } else { "0s".into() };
    }
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if h > 0 || m > 0 {
        out.push_str(&format!("{m}m"));
    }
    out.push_str(&format!("{s}s"));
    out
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

/// Go's time.ParseDuration: "300ms", "1.5h", "2h45m", "0".
pub fn parse_go_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
    if s.starts_with('-') {
        return None;
    }
    if s == "0" {
        return Some(Duration::ZERO);
    }
    if s.is_empty() {
        return None;
    }
    let mut total = 0f64;
    let mut rest = s;
    while !rest.is_empty() {
        let num_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        if num_end == 0 {
            return None;
        }
        let n: f64 = rest[..num_end].parse().ok()?;
        rest = &rest[num_end..];
        let unit_end = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let unit = &rest[..unit_end];
        rest = &rest[unit_end..];
        let mult = match unit {
            "ns" => 1e-9,
            "us" | "µs" | "μs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return None,
        };
        total += n * mult;
    }
    Some(Duration::from_secs_f64(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> FlagSet {
        let mut f = FlagSet::new();
        f.string("addr", ":2222", "");
        f.int("history", 500, "");
        f.bool("tor-exits", true, "");
        f.bool("v", false, "");
        f.duration("idle-timeout", Duration::from_secs(3600), "");
        f.float("msg-rate", 1.0, "");
        f
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn go_syntax() {
        let mut f = set();
        // what the systemd unit runs, with VIVANT_ARGS expanded
        f.parse(&args("-addr :2299 -history 50 -idle-timeout 90m -tor-exits=false -v --msg-rate=2.5")).unwrap();
        assert_eq!(f.get_string("addr"), ":2299");
        assert_eq!(f.get_int("history"), 50);
        assert_eq!(f.get_duration("idle-timeout"), Duration::from_secs(5400));
        assert!(!f.get_bool("tor-exits") && f.get_bool("v"));
        assert_eq!(f.get_float("msg-rate"), 2.5);
        assert!(set().parse(&args("-nope")).unwrap_err().contains("not defined"));
        assert_eq!(set().parse(&args("-h")).unwrap_err(), HELP);
        assert!(set().parse(&args("-history")).is_err());
        assert_eq!(set().parse(&args("-v extra -x")).unwrap(), vec!["extra", "-x"]);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_go_duration("24h0m0s"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_go_duration("1.5h"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_go_duration("300ms"), Some(Duration::from_millis(300)));
        assert_eq!(parse_go_duration("0"), Some(Duration::ZERO));
        assert_eq!(parse_go_duration("10"), None);
        assert_eq!(parse_go_duration("-1h"), None);
        assert_eq!(parse_go_duration("1d"), None);
    }
}
