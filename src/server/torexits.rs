//! Tor exit relays are shared by many people. Counting them per address
//! would let only a handful of Tor Browser users in at a time, so
//! connections from a known exit share the generous Tor bucket that
//! onion-service users already use.
//!
//! The list is Tor Project's public bulk exit list. Fetching it reveals
//! nothing about users.

use crate::{log_info, log_warn};
use std::collections::HashSet;
use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::watch;

const URL: &str = "https://check.torproject.org/torbulkexitlist";

#[derive(Clone, Default)]
pub struct ExitSet(Arc<RwLock<HashSet<IpAddr>>>);

impl ExitSet {
    pub fn contains(&self, ip: &IpAddr) -> bool {
        self.0.read().unwrap().contains(ip)
    }
    pub fn set(&self, ips: HashSet<IpAddr>) {
        *self.0.write().unwrap() = ips;
    }
}

pub fn parse(body: &str) -> HashSet<IpAddr> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
        .collect()
}

fn fetch() -> Result<HashSet<IpAddr>, String> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).user_agent("vivant").build().into();
    let mut resp = agent.get(URL).call().map_err(|e| e.to_string())?;
    let mut body = String::new();
    resp.body_mut().as_reader().take(4 << 20).read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok(parse(&body))
}

/// Downloads the list now and then every 6 hours. Failures keep the
/// previous list (or none), so it never blocks anyone.
pub async fn refresh(set: ExitSet, mut stop: watch::Receiver<bool>) {
    loop {
        match tokio::task::spawn_blocking(fetch).await {
            Ok(Ok(ips)) if ips.len() >= 100 => {
                log_info!("tor exit list updated", count = ips.len());
                set.set(ips);
            }
            Ok(Ok(ips)) => log_warn!("tor exit list: too short, ignored", count = ips.len()),
            Ok(Err(e)) => log_warn!("tor exit list: fetch failed", err = e),
            Err(_) => {}
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(6 * 3600)) => {}
            _ = stop.wait_for(|s| *s) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list() {
        let s = parse("# comment\n1.2.3.4\n\n2001:db8::1\nnot an ip\n::ffff:5.6.7.8\n");
        assert_eq!(s.len(), 3);
        assert!(s.contains(&"1.2.3.4".parse().unwrap()));
        assert!(s.contains(&"5.6.7.8".parse().unwrap()));
    }
}
