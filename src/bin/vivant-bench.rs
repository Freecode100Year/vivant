//! vivant-bench: a load generator for VIVANT. It opens N SSH sessions (line
//! mode), has some of them chat at a fixed rate and measures how long each
//! message takes to reach every other session.
//!
//! Run the server with relaxed limits first, e.g.
//!
//! ```text
//! vivant -addr 127.0.0.1:2222 -max-per-ip 100000 -max-tcp-per-ip 100000 -conn-per-min 100000 -max-clients 100000 -msg-rate 100 -msg-burst 100
//! vivant-bench -addr 127.0.0.1:2222 -clients 500 -senders 50 -rate 0.5 -duration 30s
//! ```

#[path = "../flags.rs"]
#[allow(dead_code)]
mod flags;

use russh::client;
use russh::keys::PublicKeyOrCertificate;
use russh::{ChannelMsg, Preferred};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

struct Client;

impl client::Handler for Client {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn now_ns() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64
}

#[derive(Default)]
struct Stats {
    latencies: Mutex<Vec<Duration>>,
    received: AtomicUsize,
    sent: AtomicUsize,
    failed: AtomicUsize,
    send_start: AtomicI64, // replayed room history from earlier runs is ignored
}

impl Stats {
    /// Finds every "BENCH:<ns> s" in `buf` and records its latency; returns
    /// how many bytes were consumed.
    fn scan(&self, buf: &[u8]) -> usize {
        let mut used = 0;
        let mut i = 0;
        while let Some(p) = find(&buf[i..], b"BENCH:") {
            let s = i + p + 6;
            let digits = buf[s..].iter().take_while(|b| b.is_ascii_digit()).count();
            if s + digits >= buf.len() {
                break; // number may continue in the next read
            }
            if digits == 19 && buf[s + digits] == b' ' {
                let ns: i64 = std::str::from_utf8(&buf[s..s + digits]).unwrap().parse().unwrap_or(0);
                let start = self.send_start.load(Ordering::Relaxed);
                if start != 0 && ns >= start {
                    self.received.fetch_add(1, Ordering::Relaxed);
                    let d = Duration::from_nanos((now_ns() - ns).max(0) as u64);
                    self.latencies.lock().unwrap().push(d);
                }
            }
            i = s + digits;
            used = i;
        }
        used
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

#[tokio::main]
async fn main() {
    let mut fs = flags::FlagSet::new();
    fs.string("addr", "127.0.0.1:2222", "server address");
    fs.int("clients", 200, "concurrent sessions");
    fs.int("senders", 20, "how many of them send messages");
    fs.float("rate", 0.5, "messages per second per sender");
    fs.duration("duration", Duration::from_secs(20), "how long senders chat");
    fs.duration("ramp", Duration::from_secs(5), "time to open all sessions");
    fs.bool("pty", false, "request a PTY so the server runs the full TUI (latency numbers are then approximate)");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = fs.parse(&args) {
        if e != flags::HELP {
            eprintln!("{e}");
        }
        eprint!("Usage: vivant-bench [options]\n{}", fs.defaults());
        std::process::exit(2);
    }
    let addr = fs.get_string("addr");
    let clients = fs.get_int("clients").max(1) as usize;
    let senders = (fs.get_int("senders").max(0) as usize).min(clients);
    let rate = fs.get_float("rate").max(0.001);
    let duration = fs.get_duration("duration");
    let ramp = fs.get_duration("ramp");
    let use_pty = fs.get_bool("pty");

    let config = Arc::new(client::Config { preferred: Preferred::default(), inactivity_timeout: None, ..Default::default() });
    let stats = Arc::new(Stats::default());
    let start = std::time::Instant::now();
    let mut writers: Vec<Option<mpsc::Sender<String>>> = vec![None; clients];
    let mut joins = Vec::new();
    for (i, w) in writers.iter_mut().enumerate() {
        let (tx, mut rx) = mpsc::channel::<String>(16);
        *w = Some(tx);
        let (config, addr, stats) = (config.clone(), addr.clone(), stats.clone());
        let listener = !(use_pty && i < senders); // senders' own input echo would be miscounted
        joins.push(tokio::spawn(async move {
            let ok = async {
                let mut h = client::connect(config, addr.as_str(), Client).await.ok()?;
                if !h.authenticate_none("bench").await.ok()?.success() {
                    return None;
                }
                let ch = h.channel_open_session().await.ok()?;
                if use_pty {
                    ch.request_pty(true, "xterm-256color", 120, 40, 0, 0, &[]).await.ok()?;
                }
                ch.request_shell(true).await.ok()?;
                Some((h, ch))
            }
            .await;
            let Some((h, mut ch)) = ok else {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let _h = h;
            let mut pending: Vec<u8> = Vec::new();
            loop {
                tokio::select! {
                    m = ch.wait() => match m {
                        Some(ChannelMsg::Data { data }) => {
                            if listener {
                                pending.extend_from_slice(&data);
                                let used = stats.scan(&pending);
                                pending.drain(..used);
                                if pending.len() > 256 {
                                    pending.drain(..pending.len() - 256);
                                }
                            }
                        }
                        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => return,
                        _ => {}
                    },
                    line = rx.recv() => match line {
                        Some(l) => {
                            if ch.data(l.as_bytes()).await.is_err() {
                                return;
                            }
                        }
                        None => {
                            let _ = ch.eof().await;
                            return;
                        }
                    },
                }
            }
        }));
        tokio::time::sleep(ramp / clients as u32).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await; // let join notices settle
    let connected = clients - stats.failed.load(Ordering::Relaxed);
    println!("connected {connected}/{clients} sessions in {:.3}s", start.elapsed().as_secs_f64());

    stats.send_start.store(now_ns(), Ordering::Relaxed);
    let interval = Duration::from_secs_f64(1.0 / rate);
    let stop = std::time::Instant::now() + duration;
    let mut send_tasks = Vec::new();
    for (i, w) in writers.iter().take(senders).enumerate() {
        let Some(w) = w.clone() else { continue };
        let stats = stats.clone();
        send_tasks.push(tokio::spawn(async move {
            tokio::time::sleep(interval * i as u32 / senders.max(1) as u32).await;
            let eol = if use_pty { "\r" } else { "\n" };
            let mut n = 0;
            while std::time::Instant::now() < stop {
                if w.send(format!("BENCH:{} s{i}-{n}{eol}", now_ns())).await.is_err() {
                    return;
                }
                stats.sent.fetch_add(1, Ordering::Relaxed);
                n += 1;
                tokio::time::sleep(interval).await;
            }
        }));
    }
    for t in send_tasks {
        let _ = t.await;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    drop(writers);

    let mut lat = stats.latencies.lock().unwrap().clone();
    lat.sort();
    let pct = |p: f64| -> String {
        if lat.is_empty() {
            return "0s".into();
        }
        let d = lat[((p * lat.len() as f64) as usize).min(lat.len() - 1)];
        format!("{:.1}ms", d.as_secs_f64() * 1000.0)
    };
    let listeners = if use_pty { connected.saturating_sub(senders) } else { connected };
    let sent = stats.sent.load(Ordering::Relaxed);
    let recv = stats.received.load(Ordering::Relaxed);
    let expected = sent * listeners;
    println!("sent {sent} messages, {recv} deliveries (expected {expected}, {:.1}%)", 100.0 * recv as f64 / expected.max(1) as f64);
    println!("fan-out latency  p50 {}  p95 {}  p99 {}  max {}", pct(0.50), pct(0.95), pct(0.99), pct(1.0));
    let failed = stats.failed.load(Ordering::Relaxed);
    if failed > 0 {
        eprintln!("{failed} sessions failed to connect");
    }
    std::process::exit(0);
}
