use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn shared(tcp_max: usize, tcp_all: usize) -> Arc<Shared> {
    let hub = Hub::new(crate::chat::Config::default());
    let dir = std::env::temp_dir().join(format!("vivant-srv-{}-{tcp_max}-{tcp_all}", std::process::id()));
    let opt = Options { host_key_path: dir.join("hk"), max_tcp_per_ip: tcp_max, max_tcp: tcp_all, ..Default::default() };
    let s = Server::new(hub, opt).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    s.shared
}

#[test]
fn address_keys() {
    let salt = [7u8; 32];
    let v4: IpAddr = "203.0.113.9".parse().unwrap();
    let mapped: IpAddr = "::ffff:203.0.113.9".parse().unwrap();
    assert_eq!(addr_key(&salt, v4), addr_key(&salt, mapped.to_canonical()));
    assert_eq!(addr_key(&salt, v4).len(), 24);
    // one /64 is one key; the next /64 is another
    let a = addr_key(&salt, "2001:db8:1:2::1".parse().unwrap());
    let b = addr_key(&salt, "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap());
    let c = addr_key(&salt, "2001:db8:1:3::1".parse().unwrap());
    assert_eq!(a, b);
    assert_ne!(a, c);
    // a different salt (another restart) gives unrelated keys
    assert_ne!(addr_key(&[8u8; 32], v4), addr_key(&salt, v4));
}

#[test]
fn onion_and_tor_exits_share_one_key() {
    let sh = shared(8, 100);
    let ip: IpAddr = "198.51.100.7".parse().unwrap();
    assert_eq!(key_for(&sh, true, ip), ONION_KEY);
    assert_ne!(key_for(&sh, false, ip), ONION_KEY);
    sh.tor.set([ip].into_iter().collect());
    assert_eq!(key_for(&sh, false, ip), ONION_KEY);
    assert_eq!(key_for(&sh, false, "::ffff:198.51.100.7".parse().unwrap()), ONION_KEY);
}

#[test]
fn tcp_caps() {
    let sh = shared(2, 3);
    let a1 = TcpSlot::take(&sh, "a").unwrap();
    let _a2 = TcpSlot::take(&sh, "a").unwrap();
    assert!(TcpSlot::take(&sh, "a").is_none(), "per-address cap");
    let _b = TcpSlot::take(&sh, "b").unwrap();
    assert!(TcpSlot::take(&sh, "c").is_none(), "global cap");
    drop(a1);
    assert!(TcpSlot::take(&sh, "c").is_some(), "slot released on drop");
    let onion: Vec<_> = (0..5).map(|_| TcpSlot::take(&shared(2, 100), ONION_KEY)).collect();
    assert!(onion.iter().all(|s| s.is_some()), "onion listener has its own, larger cap");
}

#[tokio::test]
async fn read_throttle_slows_floods() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let writer = tokio::spawn(async move {
        let mut c = TcpStream::connect(addr).await.unwrap();
        c.write_all(&vec![b'x'; 30_000]).await.unwrap();
    });
    let (sock, _) = l.accept().await.unwrap();
    let mut s = ConnStream::new(sock, 10_000, 10_000, Kill::default());
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; 4096];
    let mut n = 0;
    while n < 30_000 {
        let k = s.read(&mut buf).await.unwrap();
        assert!(k > 0);
        n += k;
    }
    writer.await.unwrap();
    // 10 kB at full speed, then 20 kB at 10 kB/s
    let el = start.elapsed();
    assert!(el >= Duration::from_millis(1500), "{el:?}");
}

#[tokio::test]
async fn kill_ends_reads() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let _c = TcpStream::connect(addr).await.unwrap();
    let (sock, _) = l.accept().await.unwrap();
    let kill = Kill::default();
    let mut s = ConnStream::new(sock, 0, 0, kill.clone());
    let k2 = kill.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        k2.kill();
    });
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(n, 0);
}

#[test]
fn host_key_round_trip() {
    let dir = std::env::temp_dir().join(format!("vivant-hk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = dir.join("host_ed25519");
    let k1 = load_or_create_host_key(&p).unwrap();
    let k2 = load_or_create_host_key(&p).unwrap();
    assert_eq!(k1.public_key().key_data(), k2.public_key().key_data());
    let pem = std::fs::read_to_string(&p).unwrap();
    assert!(pem.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    assert!(dir.join("host_ed25519.pub").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn authorized_keys_with_options_and_comments() {
    let dir = std::env::temp_dir().join(format!("vivant-ak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("admins");
    std::fs::write(
        &p,
        "# admins\n\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEiqTfGc9DwNGSvQ1e8nRxROeeGX8djdBi0lf0kojIvn me@laptop\nfrom=\"10.0.0.0/8\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEiqTfGc9DwNGSvQ1e8nRxROeeGX8djdBi0lf0kojIvn\n",
    )
    .unwrap();
    assert_eq!(load_authorized_keys(&p).unwrap().len(), 2);
    std::fs::write(&p, "not a key\n").unwrap();
    assert!(load_authorized_keys(&p).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
