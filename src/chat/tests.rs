use super::*;

fn drain(c: &mut Client) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(ev) = c.rx.try_recv() {
        if ev.kind == Kind::History {
            out.extend(ev.batch.iter().map(|e| (**e).clone()));
            continue;
        }
        out.push((*ev).clone());
    }
    out
}

fn has(evs: &[Event], kind: Kind, sub: &str) -> bool {
    evs.iter().any(|e| e.kind == kind && (e.text.contains(sub) || e.en.contains(sub)))
}

fn has_line(evs: &[Event], sub: &str) -> bool {
    evs.iter().any(|e| e.lines.iter().any(|l| l.contains(sub)))
}

fn closed(c: &Client) -> bool {
    c.done.borrow().is_some()
}

fn test_hub() -> Arc<Hub> {
    Hub::new(Config { msg_burst: 100.0, msg_per_second: 100.0, nick_cooldown: Duration::from_millis(1), ..Default::default() })
}

fn hub_with(f: impl FnOnce(&mut Config)) -> Arc<Hub> {
    let mut c = Config::default();
    f(&mut c);
    Hub::new(c)
}

fn conn(h: &Hub, ip: &str) -> Client {
    h.connect(ip, false).unwrap()
}

#[test]
fn broadcast_and_rooms() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    drain(&mut a);
    drain(&mut b);
    h.handle(a.id, "hello 大家好");
    assert!(has(&drain(&mut b), Kind::Chat, "hello 大家好"));
    h.handle(b.id, "/join chinese");
    drain(&mut a);
    drain(&mut b);
    h.handle(a.id, "only general");
    assert!(!has(&drain(&mut b), Kind::Chat, "only general"), "message leaked across rooms");
    h.handle(a.id, "/join nope");
    assert!(has(&drain(&mut a), Kind::Error, "No such room"));
}

#[test]
fn history_replay() {
    let h = test_hub();
    let a = conn(&h, "ip-a");
    h.handle(a.id, "first");
    h.handle(a.id, "second");
    let mut b = conn(&h, "ip-b");
    let n = drain(&mut b).iter().filter(|e| e.history && e.kind == Kind::Chat).count();
    assert_eq!(n, 2);
}

#[test]
fn dm_and_reply() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    h.handle(a.id, "/nick Alice");
    h.handle(b.id, "/nick 夜猫子");
    drain(&mut a);
    drain(&mut b);
    h.handle(a.id, "/msg 夜猫子 secret");
    assert!(has(&drain(&mut b), Kind::DmIn, "secret"));
    assert!(has(&drain(&mut a), Kind::DmOut, "secret"));
    h.handle(b.id, "/r got it");
    assert!(has(&drain(&mut a), Kind::DmIn, "got it"));
}

#[test]
fn nick_rules() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    h.handle(a.id, "/nick Ghost");
    drain(&mut a);
    drain(&mut b);
    h.handle(b.id, "/nick ghost");
    assert!(has(&drain(&mut b), Kind::Error, "taken"));
    h.handle(b.id, "/nick System");
    assert!(has(&drain(&mut b), Kind::Error, "reserved"));
    h.handle(b.id, "/nick a");
    assert!(has(&drain(&mut b), Kind::Error, "2–16"));
    h.handle(b.id, "/nick \x1b[31mred");
    assert_ne!(h.nick(b.id), "\x1b[31mred");
}

#[test]
fn sanitize_strips_escapes() {
    let got = sanitize("hi\x1b]0;pwned\x07 \x1b[2J\u{202e}evil\u{85} ok", 0);
    assert!(!got.contains(['\x1b', '\x07', '\u{202e}', '\u{85}']), "{got:?}");
    assert!(got.contains("ok"));
    assert_eq!(sanitize(&"字".repeat(900), 500).chars().count(), 500);
}

#[test]
fn flood_protection() {
    let h = hub_with(|c| {
        c.msg_burst = 3.0;
        c.msg_per_second = 0.001
    });
    let mut a = conn(&h, "ip-a");
    drain(&mut a);
    for i in 0..3 {
        h.handle(a.id, &format!("msg {i}"));
    }
    h.handle(a.id, "one too many");
    assert!(has(&drain(&mut a), Kind::Error, "too fast"));
    for i in 0..10 {
        h.handle(a.id, &format!("spam {i}"));
    }
    assert!(h.status(a.id).muted, "expected auto-mute after repeated violations");
}

#[test]
fn repeat_blocked() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    drain(&mut a);
    for _ in 0..3 {
        h.handle(a.id, "same");
    }
    assert!(has(&drain(&mut a), Kind::Error, "repeat"));
}

#[test]
fn connection_limits() {
    let h = hub_with(|c| {
        c.max_conns_per_ip = 2;
        c.conn_per_minute = 100
    });
    let c1 = h.connect("ip", false).unwrap();
    let _c2 = h.connect("ip", false).unwrap();
    assert_eq!(h.connect("ip", false).err(), Some(ConnectError::TooManyConns));
    h.disconnect(c1.id);
    assert!(h.connect("ip", false).is_ok(), "slot not freed");
    let h2 = hub_with(|c| {
        c.max_conns_per_ip = 100;
        c.conn_per_minute = 2
    });
    let _x = h2.connect("x", false);
    let _y = h2.connect("x", false);
    assert_eq!(h2.connect("x", false).err(), Some(ConnectError::ConnRate));
}

#[test]
fn private_room_lifecycle() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(a.id, "/create");
    let code = h.status(a.id).room;
    assert!(code.starts_with("p-"), "{code}");
    assert!(h.status(b.id).rooms.iter().all(|r| r.name != code), "private room visible to outsiders");
    h.handle(b.id, &format!("/join {code}"));
    drain(&mut a);
    h.handle(b.id, "psst");
    assert!(has(&drain(&mut a), Kind::Chat, "psst"));
    h.handle(a.id, "/leave");
    h.handle(b.id, "/leave");
    assert!(!h.lock().rooms.contains_key(&code), "empty private room not deleted");
}

#[test]
fn admin_kick_ban() {
    let h = test_hub();
    let adm = h.connect("ip-admin", true).unwrap();
    let bad = conn(&h, "ip-bad");
    h.handle(bad.id, "/nick Spammer");
    h.handle(bad.id, "/kick Spammer");
    assert!(!closed(&bad), "non-admin kick worked");
    h.handle(adm.id, "/ban Spammer 5");
    assert!(closed(&bad), "ban did not disconnect");
    assert_eq!(h.connect("ip-bad", false).err(), Some(ConnectError::Banned));
}

#[test]
fn history_ttl() {
    let h = hub_with(|c| {
        c.history_ttl = Duration::from_millis(10);
        c.msg_burst = 100.0;
        c.msg_per_second = 100.0
    });
    let a = conn(&h, "ip-a");
    h.handle(a.id, "old news");
    std::thread::sleep(Duration::from_millis(20));
    h.janitor();
    let mut b = conn(&h, "ip-b");
    assert!(drain(&mut b).iter().all(|e| !e.history), "expired history replayed");
}

#[test]
fn ignore() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    h.handle(a.id, "/nick Annoying");
    h.handle(b.id, "/ignore Annoying");
    drain(&mut a);
    drain(&mut b);
    h.handle(a.id, "blah");
    h.handle(a.id, &format!("/msg {} psst", h.nick(b.id)));
    let evs = drain(&mut b);
    assert!(!has(&evs, Kind::Chat, "blah") && !has(&evs, Kind::DmIn, "psst"));
    assert!(has(&drain(&mut a), Kind::DmOut, "psst"), "sender should not learn they are ignored");
    h.handle(b.id, "/unignore Annoying");
    h.handle(a.id, "blah again");
    assert!(has(&drain(&mut b), Kind::Chat, "blah again"));
}

#[test]
fn report() {
    let h = test_hub();
    let mut adm = h.connect("ip-admin", true).unwrap();
    let mut a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(b.id, "/nick Troll");
    drain(&mut adm);
    h.handle(a.id, "/report Troll spamming links");
    let evs = drain(&mut adm);
    assert!(
        evs.iter().any(|e| e.kind == Kind::Notice && e.lines.len() == 1 && e.lines[0].contains("Troll") && e.lines[0].contains("spamming")),
        "admin not notified: {evs:?}"
    );
    h.handle(a.id, "/reports");
    assert!(!has(&drain(&mut a), Kind::Info, "recent reports"), "non-admin can read reports");
}

#[test]
fn timezone() {
    for (input, want) in [("+8", 8 * 3600), ("-5", -5 * 3600), ("+05:30", 5 * 3600 + 1800), ("UTC+9", 9 * 3600)] {
        assert_eq!(parse_zone(input).unwrap(), Zone::Fixed(want), "{input}");
    }
    assert_eq!(parse_zone("utc").unwrap(), Zone::Utc);
    assert!(parse_zone("../../etc/passwd").is_err());
    assert!(parse_zone("+99").is_err());
    let h = test_hub();
    let a = conn(&h, "ip-a");
    h.handle(a.id, "/tz +8");
    assert_eq!(h.loc(a.id), Zone::Fixed(8 * 3600));
    for z in ["utc+8", "gmt-5", "UTC+05:30"] {
        assert!(parse_zone(z).is_ok(), "{z}");
    }
}

#[test]
fn idle_kick() {
    let h = hub_with(|c| c.idle_kick = Duration::from_millis(40));
    let a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    std::thread::sleep(Duration::from_millis(30));
    h.handle(b.id, "/who");
    std::thread::sleep(Duration::from_millis(20));
    h.janitor();
    assert!(closed(&a), "idle client not dropped");
    assert!(!closed(&b), "active client dropped");
}

#[test]
fn stats_has_no_identities() {
    let h = test_hub();
    let _a = conn(&h, "ip-a");
    let st = h.stats();
    assert_eq!((st.online, st.rooms), (1, 5));
    let json = serde_json::to_string(&st).unwrap();
    assert!(!json.contains("anonymous_") && !json.contains("ip-a"));
}

#[test]
fn onion_bucket() {
    let h = hub_with(|c| {
        c.max_conns_per_ip = 2;
        c.onion_max_conns = 5;
        c.onion_per_min = 100
    });
    let mut keep = Vec::new();
    for i in 0..5 {
        keep.push(h.connect(ONION_KEY, false).unwrap_or_else(|e| panic!("onion session {i} refused: {e}")));
    }
    assert_eq!(h.connect(ONION_KEY, false).err(), Some(ConnectError::TooManyConns));
    let adm = h.connect("ip-admin", true).unwrap();
    let victim = &keep[0];
    h.handle(adm.id, &format!("/ban {}", h.nick(victim.id)));
    assert!(closed(victim));
    assert!(h.connect(ONION_KEY, false).is_ok(), "banning one Tor user locked out all Tor users");
}

#[test]
fn unread_counts() {
    let h = test_hub();
    let a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(b.id, "/join chinese");
    h.handle(b.id, "一");
    h.handle(b.id, "二");
    let unread = |id: u64, room: &str| h.status(id).rooms.into_iter().find(|r| r.name == room).unwrap();
    assert_eq!(unread(a.id, "chinese").unread, 2);
    assert_eq!(unread(b.id, "chinese").unread, 0);
    h.handle(a.id, "/join chinese");
    h.handle(a.id, "/leave");
    assert_eq!(unread(a.id, "chinese").unread, 0);
    h.handle(b.id, "三");
    assert_eq!(unread(a.id, "chinese").unread, 1);
}

#[test]
fn cross_room_mention() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(a.id, "/nick 夜猫子");
    h.handle(b.id, "/nick Bob");
    h.handle(b.id, "/join chinese");
    drain(&mut a);
    h.handle(b.id, "@夜猫子你好，来中文频道");
    assert!(has(&drain(&mut a), Kind::Mention, "@夜猫子你好"));
    assert!(h.status(a.id).rooms.iter().any(|r| r.name == "chinese" && r.mentioned));
    h.handle(a.id, "/join chinese");
    drain(&mut a);
    h.handle(b.id, "@夜猫子 again");
    assert!(!has(&drain(&mut a), Kind::Mention, "again"));
    h.handle(a.id, "/leave");
    h.handle(a.id, "/ignore Bob");
    drain(&mut a);
    h.handle(b.id, "@夜猫子 hello?");
    assert!(!has(&drain(&mut a), Kind::Mention, "hello?"));
}

#[test]
fn mention_never_leaks_private_room() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(a.id, "/nick Alice");
    h.handle(b.id, "/create");
    drain(&mut a);
    h.handle(b.id, "@Alice secret room talk");
    assert!(!has(&drain(&mut a), Kind::Mention, "secret"));
}

#[test]
fn ban_list_unban_and_persist_api() {
    let h = test_hub();
    let mut adm = h.connect("ip-admin", true).unwrap();
    let bad = conn(&h, "deadbeefcafe0001");
    let (_, v0) = h.bans();
    h.handle(adm.id, &format!("/ban {} 30", h.nick(bad.id)));
    let (bans, v1) = h.bans();
    assert!(bans.len() == 1 && v1 != v0);
    drain(&mut adm);
    h.handle(adm.id, "/bans");
    assert!(has(&drain(&mut adm), Kind::Info, "1 active bans"));
    h.handle(adm.id, "/unban deadbeef");
    assert!(h.bans().0.is_empty());
    let h2 = test_hub();
    let now = Utc::now();
    let hour = chrono::Duration::hours(1);
    let n = h2.load_bans(HashMap::from([("k1".to_string(), now + hour), ("old".into(), now - hour), (ONION_KEY.into(), now + hour)]));
    assert_eq!(n, 1);
    assert_eq!(h2.connect("k1", false).err(), Some(ConnectError::Banned));
}

#[test]
fn forget_erases_own_history_only() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    let b = conn(&h, "ip-b");
    h.handle(a.id, "secret from a");
    h.handle(b.id, "hello from b");
    h.handle(a.id, "/join chinese");
    h.handle(a.id, "another secret");
    drain(&mut a);
    h.handle(a.id, "/forget");
    assert!(has(&drain(&mut a), Kind::Info, "删除你的 2 条消息"));
    let mut c = conn(&h, "ip-c");
    let evs = drain(&mut c);
    assert!(!evs.iter().any(|e| e.history && e.text.contains("secret")));
    assert!(has(&evs, Kind::Chat, "hello from b"));
}

#[test]
fn privacy_reflects_config() {
    let h = hub_with(|c| {
        c.history_size = 20;
        c.history_ttl = Duration::from_secs(1800);
        c.persist_bans = true;
        c.tor_enabled = true
    });
    let mut a = conn(&h, "ip-a");
    drain(&mut a);
    h.handle(a.id, "/privacy");
    let evs = drain(&mut a);
    assert_eq!(evs.len(), 1);
    let all = evs[0].lines.join("\n");
    for want in ["最多 20 条", "30 分钟", "已开启持久化", ".onion", "不写入日志", "能看到连接来源 IP"] {
        assert!(all.contains(want), "missing {want}:\n{all}");
    }
    let h2 = test_hub();
    let mut b = conn(&h2, "ip-b");
    drain(&mut b);
    h2.handle(b.id, "/privacy");
    let all = drain(&mut b)[0].lines.join("\n");
    assert!(all.contains("仅在内存") && all.contains("未开启"));
}

#[test]
fn help_is_short_by_default() {
    let h = test_hub();
    let mut a = conn(&h, "ip-a");
    drain(&mut a);
    h.handle(a.id, "/help");
    let short = drain(&mut a);
    assert!(short.len() == 1 && short[0].lines.len() <= 8 && short[0].lines.join("\n").contains("/help all"));
    h.handle(a.id, "/help all");
    let all = drain(&mut a)[0].lines.join("\n");
    for cmd in ["/forget", "/privacy", "/ignore", "/create", "/tz"] {
        assert!(all.contains(cmd), "/help all missing {cmd}");
    }
    assert!(!all.contains("/kick"), "admin commands shown to a non-admin");
}

#[test]
fn presence_summary_counts() {
    let h = hub_with(|c| c.quiet_above = 1);
    let mut a = conn(&h, "a");
    let _b = conn(&h, "b");
    drain(&mut a);
    let c = conn(&h, "c");
    let _d = conn(&h, "d");
    h.disconnect(c.id);
    h.flush_presence();
    assert!(has(&drain(&mut a), Kind::System, "2 人进入、1 人离开"));
}

#[test]
fn moderation_never_collapsed() {
    let h = hub_with(|c| c.quiet_above = 1);
    let mut adm = h.connect("adm", true).unwrap();
    let _x = conn(&h, "x");
    let v = conn(&h, "v");
    drain(&mut adm);
    h.handle(adm.id, &format!("/kick {}", h.nick(v.id)));
    assert!(has(&drain(&mut adm), Kind::System, "was kicked"));
}

#[test]
fn bell_toggle() {
    let h = test_hub();
    let a = conn(&h, "a");
    assert!(h.bell_on(a.id) && h.status(a.id).bell);
    h.handle(a.id, "/bell off");
    assert!(!h.bell_on(a.id));
    h.handle(a.id, "/bell on");
    assert!(h.bell_on(a.id));
}

#[test]
fn theme_command() {
    let h = test_hub();
    let a = conn(&h, "a");
    assert_eq!(h.status(a.id).theme, THEME_GREEN);
    for (cmd, want) in [
        ("/theme", THEME_DARK),
        ("/theme", THEME_LIGHT),
        ("/theme", THEME_GREEN),
        ("/theme black", THEME_DARK),
        ("/theme 护眼", THEME_GREEN),
        ("/theme white", THEME_LIGHT),
        ("/bg black", THEME_DARK),
        ("/theme purple", THEME_DARK),
    ] {
        h.handle(a.id, cmd);
        assert_eq!(h.status(a.id).theme, want, "{cmd}");
    }
    h.set_theme(a.id, "white");
    assert_eq!(h.status(a.id).theme, THEME_LIGHT);
    h.set_theme(a.id, "someuser");
    assert_eq!(h.status(a.id).theme, THEME_LIGHT);
}

#[test]
fn bilingual() {
    let h = test_hub();
    let mut a = conn(&h, "a");
    let evs = drain(&mut a);
    assert!(evs[0].text.contains("欢迎") && evs[0].en.contains("Welcome"));
    for cmd in [
        "/help",
        "/help all",
        "/rooms",
        "/who",
        "/privacy",
        "/bell",
        "/tz",
        "/theme",
        "/theme nope",
        "/join nope",
        "/nick x",
        "/frobnicate",
        "/lang en",
    ] {
        h.handle(a.id, cmd);
        for ev in drain(&mut a) {
            if ev.kind != Kind::Chat && ev.kind != Kind::Action {
                assert!(!ev.en.is_empty(), "{cmd}: event without English: {:?}", ev.text);
            }
            if !ev.lines.is_empty() && ev.lines_en.is_empty() && cmd != "/who" && cmd != "/rooms" {
                assert!(ev.lines.join("\n").contains(" · "), "{cmd}: lines in one language only: {:?}", ev.lines);
            }
        }
    }
}

#[test]
fn join_hints() {
    let h = test_hub();
    let mut a = conn(&h, "a");
    assert!(has_line(&drain(&mut a), "只有你在线"));
    let mut b = conn(&h, "b");
    h.handle(b.id, "/join english");
    assert!(has_line(&drain(&mut b), "#general 有 1 人"));
    h.handle(b.id, "/join general");
    assert!(has_line(&drain(&mut b), "还没有消息"));
    h.handle(a.id, "/create");
    drain(&mut a);
    h.handle(b.id, "/join english");
    assert!(has_line(&drain(&mut b), "这里暂时只有你"));
}

#[test]
fn free_nick_never_spins() {
    let mut by_nick = HashMap::new();
    for i in 0..10000 {
        by_nick.insert(format!("anonymous_{i:04}"), 0);
    }
    let n = free_nick(&by_nick);
    assert!(valid_nick(&n) && n.len() == "anonymous_00000".len(), "{n}");
}

#[test]
fn random_codes() {
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        let c = random_code(10);
        assert!(c.len() == 10 && c.bytes().all(|b| CODE_ALPHABET.contains(&b)) && seen.insert(c));
    }
}

#[test]
fn default_history_keeps_a_day() {
    let h = Hub::new(Config::default());
    assert_eq!((h.cfg.history_ttl, h.cfg.history_size), (Duration::from_secs(86400), 500));
    let a = conn(&h, "ip-a");
    h.handle(a.id, "yesterday");
    let set_age = |hours: i64| {
        let mut st = h.lock();
        let lobby = st.public[0].clone();
        let r = st.rooms.get_mut(&lobby).unwrap();
        let mut e = (*r.history[0]).clone();
        e.time = Utc::now() - chrono::Duration::hours(hours);
        r.history[0] = Arc::new(e);
    };
    set_age(23);
    h.janitor();
    let mut b = conn(&h, "ip-b");
    assert!(has(&drain(&mut b), Kind::Chat, "yesterday"), "23h-old message was not kept");
    set_age(25);
    h.janitor();
    let mut c = conn(&h, "ip-c");
    assert!(!has(&drain(&mut c), Kind::Chat, "yesterday"), "25h-old message was kept");
}

#[test]
fn long_history_replay_not_dropped() {
    let h = hub_with(|c| {
        c.msg_burst = 1e6;
        c.msg_per_second = 1e6
    });
    let a = conn(&h, "ip-a");
    for i in 0..400 {
        h.handle(a.id, &format!("msg {i}"));
    }
    let mut b = conn(&h, "ip-b");
    let evs = drain(&mut b);
    assert_eq!(evs.iter().filter(|e| e.history && e.kind == Kind::Chat).count(), 400);
    assert!(has(&evs, Kind::Info, "Now in #general"));
}

#[test]
fn history_caps() {
    let h = hub_with(|c| {
        c.history_size = 40;
        c.msg_burst = 1e6;
        c.msg_per_second = 1e6
    });
    let a = conn(&h, "ip-a");
    for i in 0..100 {
        h.handle(a.id, &format!("public {i}"));
    }
    let mut b = conn(&h, "ip-b");
    assert_eq!(drain(&mut b).iter().filter(|e| e.history).count(), 40);
    assert_eq!(h.cfg.private_history_size, 40);
}

#[test]
fn replay_skips_ignored() {
    let h = test_hub();
    let a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    h.handle(a.id, "noise from a");
    h.handle(b.id, &format!("/ignore {}", h.nick(a.id)));
    h.handle(b.id, "/join chinese");
    drain(&mut b);
    h.handle(b.id, "/join general");
    assert!(!has(&drain(&mut b), Kind::Chat, "noise from a"));
}

#[test]
fn dur_texts() {
    assert_eq!(dur_text(Duration::from_secs(86400)).1, "24h");
    assert_eq!(dur_text(Duration::from_secs(1800)).1, "30m");
    assert_eq!(dur_text(Duration::from_secs(90)).1, "1m30s");
}

#[test]
fn slow_reader_never_blocks_others() {
    // a client that never reads must not stall delivery to anyone else
    let h = hub_with(|c| {
        c.msg_burst = 1e6;
        c.msg_per_second = 1e6
    });
    let _stuck = conn(&h, "ip-stuck");
    let mut b = conn(&h, "ip-b");
    let a = conn(&h, "ip-a");
    drain(&mut b);
    for i in 0..1000 {
        h.handle(a.id, &format!("m{i}"));
        if i % 100 == 0 {
            drain(&mut b);
        }
    }
    h.handle(a.id, "last one");
    assert!(has(&drain(&mut b), Kind::Chat, "last one"));
}

#[test]
fn literal_slash() {
    let h = test_hub();
    let a = conn(&h, "ip-a");
    let mut b = conn(&h, "ip-b");
    drain(&mut b);
    h.handle(a.id, "//help me");
    assert!(has(&drain(&mut b), Kind::Chat, "/help me"));
}
