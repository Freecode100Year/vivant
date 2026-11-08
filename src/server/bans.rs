//! Ban persistence (opt-in, -ban-file).
//!
//! Bans are keyed by an HMAC of the client address. By default the HMAC key
//! lives only in memory and changes on every restart, so nothing on disk can
//! ever be linked to an address — but bans are forgotten on restart too.
//! With a ban file the HMAC key is kept next to it (mode 0600) so bans
//! survive restarts and upgrades. Trade-off, documented in the README: anyone
//! who obtains BOTH files could brute-force which IPv4 addresses are banned
//! (only banned addresses are ever written; nothing about other users is).
//!
//! The file formats are the Go version's, so an upgrade keeps the bans.

use crate::chat::Hub;
use crate::log_error;
use chrono::{DateTime, SecondsFormat, Utc};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

#[derive(serde::Serialize, serde::Deserialize)]
struct BanFile {
    version: u32,
    #[serde(default)]
    bans: BTreeMap<String, String>,
}

pub fn active(bans: &HashMap<String, DateTime<Utc>>) -> usize {
    let now = Utc::now();
    bans.values().filter(|t| **t > now).count()
}

pub fn read_bans(path: &Path) -> Result<HashMap<String, DateTime<Utc>>, String> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    let f: BanFile = serde_json::from_slice(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = HashMap::new();
    for (k, v) in f.bans {
        let t = DateTime::parse_from_rfc3339(&v).map_err(|e| format!("{}: {k}: {e}", path.display()))?;
        out.insert(k, t.with_timezone(&Utc));
    }
    Ok(out)
}

pub fn write_bans(path: &Path, bans: &HashMap<String, DateTime<Utc>>) -> std::io::Result<()> {
    let f = BanFile { version: 1, bans: bans.iter().map(|(k, v)| (k.clone(), v.to_rfc3339_opts(SecondsFormat::AutoSi, true))).collect() };
    let mut data = serde_json::to_vec_pretty(&f).map_err(std::io::Error::other)?;
    data.push(b'\n');
    write_file_atomic(path, &data)
}

/// Reads the persistent HMAC key, creating it if missing.
pub fn load_or_create_salt(path: &Path) -> Result<Vec<u8>, String> {
    match std::fs::read(path) {
        Ok(b) if b.len() == 32 => return Ok(b),
        Ok(b) => return Err(format!("{}: unexpected length {}", path.display(), b.len())),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        Err(_) => {}
    }
    let mut b = vec![0u8; 32];
    crate::chat::random_bytes(&mut b);
    write_file_atomic(path, &b).map_err(|e| e.to_string())?;
    Ok(b)
}

/// Writes via a temp file + rename (mode 0600, directory 0700) so a crash
/// never leaves a truncated file behind.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    create_dir_private(dir)?;
    let mut name = [0u8; 8];
    crate::chat::random_bytes(&mut name);
    let tmp: PathBuf = dir.join(format!(".tmp-{}", name.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    let res = (|| {
        let mut f = open_private(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(unix)]
fn open_private(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p)
}

#[cfg(not(unix))]
fn open_private(p: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(p)
}

#[cfg(unix)]
fn create_dir_private(d: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if d.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d)
}

#[cfg(not(unix))]
fn create_dir_private(d: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(d)
}

/// Saves the hub's bans whenever they change, and once more on shutdown.
pub async fn persist(hub: Arc<Hub>, path: PathBuf, mut stop: watch::Receiver<bool>) {
    let mut saved = 0u64;
    let save = |saved: &mut u64| {
        let (bans, ver) = hub.bans();
        if ver == *saved {
            return;
        }
        match write_bans(&path, &bans) {
            Ok(()) => *saved = ver,
            Err(e) => log_error!("saving bans failed", err = e),
        }
    };
    let mut t = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = t.tick() => save(&mut saved),
            _ = stop.wait_for(|s| *s) => {
                save(&mut saved);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_go_format_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("vivant-bans-{}", std::process::id()));
        let path = dir.join("bans.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_bans(&path).unwrap().is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        // what the Go version writes
        std::fs::write(&path, r#"{"version":1,"bans":{"0a1b2c3d4e5f60718293a4b5":"2999-01-02T03:04:05.123456789-05:00"}}"#).unwrap();
        let b = read_bans(&path).unwrap();
        assert_eq!(b.len(), 1);
        write_bans(&path, &b).unwrap();
        assert_eq!(read_bans(&path).unwrap(), b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let salt_path = dir.join("ban.key");
        let s1 = load_or_create_salt(&salt_path).unwrap();
        assert_eq!(load_or_create_salt(&salt_path).unwrap(), s1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
