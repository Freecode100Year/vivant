//! Minimal structured logging to stderr, in the same `key=value` text format
//! the Go version wrote (journald keeps it). Nothing that could identify a
//! user — addresses, nicknames, message text — is ever passed here; the SSH
//! library's own logging is not wired up at all, so it can't leak either.

use chrono::{Local, SecondsFormat};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

pub fn set_debug(on: bool) {
    DEBUG.store(on, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

fn quote(v: &str) -> String {
    if !v.is_empty() && v.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '=') { v.to_string() } else { format!("{v:?}") }
}

pub fn emit(level: &str, msg: &str, kv: &[(&str, String)]) {
    let mut line = format!("time={} level={level} msg={}", Local::now().to_rfc3339_opts(SecondsFormat::Millis, false), quote(msg));
    for (k, v) in kv {
        let _ = write!(line, " {k}={}", quote(v));
    }
    eprintln!("{line}");
}

#[macro_export]
macro_rules! log_at {
    ($lvl:expr, $msg:expr $(, $k:ident = $v:expr)* $(,)?) => {
        $crate::log::emit($lvl, $msg, &[$((stringify!($k), $v.to_string())),*])
    };
}
#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::log_at!("INFO", $($t)*) }; }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::log_at!("WARN", $($t)*) }; }
#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::log_at!("ERROR", $($t)*) }; }
#[macro_export]
macro_rules! log_debug {
    ($($t:tt)*) => { if $crate::log::debug_enabled() { $crate::log_at!("DEBUG", $($t)*) } };
}
