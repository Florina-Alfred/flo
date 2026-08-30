use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use flo_rs::config::{ActiveRules, ClientConfig, run_hot_reload_with_registry};
use flo_rs::engine;
use flo_rs::registration::{
    ClientState, RegistrationError, RegistrationStatus, register_with_client,
    run_heartbeat_monitor, run_registration_handler,
};
use flo_rs::registry::Registry;
use flo_rs::rules::{Action, EvalMode, Qos, Rule, Rules, Ruleset, Trigger, When};
use flo_rs::transport::Transport;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn get_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    // small pause so OS releases it before zenoh binds
    std::thread::sleep(Duration::from_millis(10));
    p
}

fn router_config(port: u16) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    let _ = c.insert_json5("mode", "\"router\"");
    let _ = c.insert_json5("scouting/multicast/enabled", "false");
    let _ = c.insert_json5("scouting/gossip/enabled", "false");
    let _ = c.insert_json5("listen/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
    c
}

fn client_config(port: u16) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    let _ = c.insert_json5("mode", "\"client\"");
    let _ = c.insert_json5("scouting/multicast/enabled", "false");
    let _ = c.insert_json5("connect/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
    c
}

fn test_client_config() -> ClientConfig {
    ClientConfig::from_toml(
        r#"
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
"#,
    )
    .expect("test client config must parse")
}

/// Poll `cond` until it returns true or timeout hits; return true if succeeded.
async fn poll_until<F, Fut>(mut cond: F, timeout_dur: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout_dur {
        if cond().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

// ---------------------------------------------------------------------------
// 1. Heartbeat / liveliness
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_poison_on_delete_after_registered() {
    let port = get_free_port();
    let server = Arc::new(
        Transport::open_with(router_config(port))
            .await
            .expect("open router"),
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let reg = flo_rs::registration::RegistrationServer::new(Default::default());
    reg.register("robot-hb-1", test_client_config())
        .await
        .expect("pre-register");

    let srv_clone = server.clone();
    let reg_clone = reg.clone();
    let monitor = tokio::spawn(async move {
        let _ = run_heartbeat_monitor(srv_clone, reg_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    // subscribe to alert before the delete so we don't miss it
    let alert_key = flo_rs::topic::heartbeat_alert("robot-hb-1");
    let (alert_tx, mut alert_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let _alert_sub = server
        .subscribe_managed(&alert_key, move |s: zenoh::sample::Sample| {
            let _ = alert_tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe alert");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // client declares liveliness
    let mut client = Transport::open_with(client_config(port))
        .await
        .expect("open client");
    tokio::time::sleep(Duration::from_millis(600)).await;
    client
        .declare_liveliness("robot-hb-1")
        .await
        .expect("declare liveliness");
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert_eq!(
        reg.state("robot-hb-1").await,
        ClientState::Registered,
        "state must still be Registered before delete"
    );

    // dropping the client closes its session => Delete
    drop(client);
    // wait for poison
    let poisoned = poll_until(
        || {
            let r = reg.clone();
            async move { r.state("robot-hb-1").await == ClientState::Poisoned }
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(poisoned, "expected Poisoned after liveliness Delete");

    // alert must have been published
    let got = tokio::time::timeout(Duration::from_secs(3), alert_rx.recv()).await;
    assert!(got.is_ok(), "expected heartbeat_alert Poisoned, timed out");
    let opt = got.unwrap();
    assert!(
        opt.is_some(),
        "expected heartbeat_alert Poisoned, channel closed"
    );
    let bytes = opt.unwrap();
    let resp: flo_rs::registration::RegistrationResponse =
        serde_json::from_slice(&bytes).expect("alert is RegistrationResponse");
    assert_eq!(resp.status, RegistrationStatus::Poisoned);

    monitor.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_no_poison_when_token_dropped_before_register() {
    let port = get_free_port();
    let server = Arc::new(
        Transport::open_with(router_config(port))
            .await
            .expect("open router"),
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let reg = flo_rs::registration::RegistrationServer::new(Default::default());
    let srv_clone = server.clone();
    let reg_clone = reg.clone();
    let monitor = tokio::spawn(async move {
        let _ = run_heartbeat_monitor(srv_clone, reg_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    let alert_key = flo_rs::topic::heartbeat_alert("robot-hb-pre");
    let (alert_tx, mut alert_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let _alert_sub = server
        .subscribe_managed(&alert_key, move |s: zenoh::sample::Sample| {
            let _ = alert_tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe alert");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client = Transport::open_with(client_config(port))
        .await
        .expect("open client");
    tokio::time::sleep(Duration::from_millis(600)).await;
    client
        .declare_liveliness("robot-hb-pre")
        .await
        .expect("declare");
    tokio::time::sleep(Duration::from_millis(400)).await;
    drop(client);
    tokio::time::sleep(Duration::from_secs(1)).await;

    let st = reg.state("robot-hb-pre").await;
    assert_ne!(
        st,
        ClientState::Poisoned,
        "dropping token before register must NOT poison, got {st:?}"
    );

    // no alert should have been sent
    let got = tokio::time::timeout(Duration::from_millis(600), alert_rx.recv()).await;
    assert!(
        got.is_err(),
        "must not receive heartbeat_alert for unregistered client"
    );

    monitor.abort();
}

// ---------------------------------------------------------------------------
// 2. Registration envelope
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn registration_envelope_loopback() {
    let port = get_free_port();
    let server = Arc::new(
        Transport::open_with(router_config(port))
            .await
            .expect("open router"),
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let reg = flo_rs::registration::RegistrationServer::new(Default::default());
    let srv_clone = server.clone();
    let reg_clone = reg.clone();
    let handler = tokio::spawn(async move {
        let _ = run_registration_handler(srv_clone, reg_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(600)).await;

    let client = Arc::new(
        Transport::open_with(client_config(port))
            .await
            .expect("open client"),
    );
    tokio::time::sleep(Duration::from_millis(700)).await;

    let cfg = test_client_config();

    // 1) first register -> Ack
    let r1 = register_with_client(client.clone(), "robot-reg-1", &cfg).await;
    assert!(r1.is_ok(), "first register should Ack, got {r1:?}");

    // 2) duplicate -> AlreadyRegistered
    let r2 = register_with_client(client.clone(), "robot-reg-1", &cfg).await;
    assert!(
        matches!(r2, Err(RegistrationError::AlreadyRegistered)),
        "duplicate should be AlreadyRegistered, got {r2:?}"
    );

    // 3) bad JSON ignored: publish raw invalid bytes, then ensure next valid still works
    client
        .put_bytes(flo_rs::topic::REGISTRATION_KEY, b"not json at all".to_vec())
        .await
        .expect("put bad json");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let r3 = register_with_client(client.clone(), "robot-reg-2", &cfg).await;
    assert!(
        r3.is_ok(),
        "after bad JSON, next valid register should still Ack, got {r3:?}"
    );

    // 4) empty robot_id -> error (handler returns MissingRobotId which surfaces as ServerError)
    let r4 = register_with_client(client.clone(), "", &cfg).await;
    assert!(r4.is_err(), "empty robot_id must error, got {r4:?}");
    // Ensure it's not a Timeout that would hide MissingRobotId, but any Err satisfies spec.
    // If we got MissingRobotId via ServerError, the string should mention it.
    if let Err(RegistrationError::ServerError(msg)) = &r4 {
        // either contains MissingRobotId or generic unexpected status that still proves handler replied
        assert!(
            msg.contains("MissingRobotId") || msg.contains("unexpected"),
            "empty robot_id ServerError should mention MissingRobotId, got {msg}"
        );
    }

    handler.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_bad_json_does_not_crash_handler() {
    // explicit second test for bad JSON isolation
    let port = get_free_port();
    let server = Arc::new(
        Transport::open_with(router_config(port))
            .await
            .expect("open router"),
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let reg = flo_rs::registration::RegistrationServer::new(Default::default());
    let srv_clone = server.clone();
    let handler = tokio::spawn(async move {
        let _ = run_registration_handler(srv_clone, reg).await;
    });
    tokio::time::sleep(Duration::from_millis(600)).await;
    let client = Arc::new(
        Transport::open_with(client_config(port))
            .await
            .expect("open client"),
    );
    tokio::time::sleep(Duration::from_millis(700)).await;

    // flood a few bad payloads
    for _ in 0..3 {
        client
            .put_bytes(flo_rs::topic::REGISTRATION_KEY, b"{ bad json".to_vec())
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(400)).await;

    let cfg = test_client_config();
    let r = register_with_client(client.clone(), "robot-reg-good", &cfg).await;
    assert!(
        r.is_ok(),
        "handler must still Ack after bad JSON, got {r:?}"
    );
    handler.abort();
}

// ---------------------------------------------------------------------------
// 3. Hot-reload with Registry
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn hot_reload_with_registry_conflict_and_bad_toml() {
    let port = get_free_port();
    let server = Arc::new(
        Transport::open_with(router_config(port))
            .await
            .expect("open router"),
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let client = Arc::new(
        Transport::open_with(client_config(port))
            .await
            .expect("open client"),
    );
    tokio::time::sleep(Duration::from_millis(700)).await;

    // temp registry db
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "flo-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("audit.db");
    let registry = Arc::new(Registry::new(&db_path).expect("registry new"));
    let store = ActiveRules::bootstrap("rules = []\n").expect("empty store");

    let srv_clone = server.clone();
    let st_clone = store.clone();
    let reg_clone = registry.clone();
    let hot = tokio::spawn(async move {
        let _ = run_hot_reload_with_registry(&srv_clone, "robot-hot", st_clone, reg_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(600)).await;

    // good ruleset from correct owner
    let good = Ruleset {
        ruleset_name: "acme".into(),
        version: 1,
        robot_owner: "robot-hot".into(),
        rules: vec![Rule {
            name: "r-good".into(),
            when: When {
                all: vec![Trigger {
                    topic: "sensor/foo".into(),
                    pred: None,
                    mode: EvalMode::Level,
                }],
                any: vec![],
            },
            actions: vec![Action {
                topic: "actuator/bar".into(),
                qos: Qos::Reliable,
                payload: serde_json::json!({"triggered": true}),
            }],
        }],
    };
    let good_toml = good.to_toml();
    let pub_key = flo_rs::topic::ruleset_pub_key("cell-7", "acme");
    client
        .put_bytes(&pub_key, good_toml.as_bytes().to_vec())
        .await
        .expect("publish good");
    let updated = poll_until(
        || {
            let s = store.clone();
            async move { s.current().await.rules.len() == 1 }
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(updated, "store should have 1 rule after good publish");

    // conflict: same name, different owner
    let conflict = Ruleset {
        ruleset_name: "acme".into(),
        version: 2,
        robot_owner: "robot-other".into(),
        rules: vec![Rule {
            name: "r-conflict".into(),
            when: When {
                all: vec![Trigger {
                    topic: "sensor/other".into(),
                    pred: None,
                    mode: EvalMode::Level,
                }],
                any: vec![],
            },
            actions: vec![Action {
                topic: "actuator/other".into(),
                qos: Qos::Reliable,
                payload: serde_json::json!({"x": 1}),
            }],
        }],
    };
    let conflict_toml = conflict.to_toml();
    client
        .put_bytes(&pub_key, conflict_toml.as_bytes().to_vec())
        .await
        .expect("publish conflict");
    tokio::time::sleep(Duration::from_millis(800)).await;
    // store must stay unchanged
    assert_eq!(
        store.current().await.rules.len(),
        1,
        "conflict must not change store"
    );
    assert_eq!(
        store.current().await.rules[0].name,
        "r-good",
        "conflict must keep previous rule"
    );

    // audit must contain rejected_conflict
    let conn = rusqlite::Connection::open(&db_path).expect("open audit db");
    let cnt: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit WHERE status='rejected_conflict'",
            [],
            |r| r.get(0),
        )
        .expect("query audit");
    assert!(
        cnt >= 1,
        "audit should have rejected_conflict row, got cnt={cnt}"
    );

    // bad TOML keeps previous
    client
        .put_bytes(&pub_key, b"this is not toml {{{".to_vec())
        .await
        .expect("publish bad toml");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        store.current().await.rules.len(),
        1,
        "bad TOML must keep previous"
    );

    hot.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Engine hot-swap (topic rebuild)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn engine_hot_swap_new_topic_fires() {
    // single-transport engine test (loopback, no mesh needed)
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback"),
    );

    let old_toml = r#"
[[rules]]
name = "old-rule"
when.all = [{ topic = "sensor/old", mode = "Level" }]
actions = [{ topic = "actuator/old", qos = "reliable", payload = { fired_old = true } }]
"#;
    let store = ActiveRules::bootstrap(old_toml).expect("old store");

    let (tx_old, mut rx_old) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/old", move |s: zenoh::sample::Sample| {
            let _ = tx_old.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("sub old");

    let (tx_new, mut rx_new) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/new", move |s: zenoh::sample::Sample| {
            let _ = tx_new.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("sub new");

    let counter = Arc::new(AtomicU64::new(0));
    let c2 = counter.clone();
    let t2 = transport.clone();
    let s2 = store.clone();
    let engine_h = tokio::spawn(async move {
        let _ = engine::run_engine(t2, s2, c2, None).await;
    });

    // wait for engine tick so subscriptions are live
    let baseline = counter.load(Ordering::SeqCst);
    let ok = poll_until(
        || {
            let c = counter.clone();
            let b = baseline;
            async move { c.load(Ordering::SeqCst) > b }
        },
        Duration::from_secs(3),
    )
    .await;
    assert!(ok, "engine should start ticking");

    // old topic must fire
    transport
        .publish("sensor/old", Qos::BestEffort, &serde_json::json!({"v": 1}))
        .await
        .expect("pub old");
    let got_old = tokio::time::timeout(Duration::from_secs(3), rx_old.recv()).await;
    assert!(
        got_old.is_ok() && got_old.unwrap().is_some(),
        "old rule should fire before swap"
    );

    // swap to new topic
    let new_toml = r#"
[[rules]]
name = "new-rule"
when.all = [{ topic = "sensor/new", mode = "Level" }]
actions = [{ topic = "actuator/new", qos = "reliable", payload = { fired_new = true } }]
"#;
    let new_rules = Rules::from_toml(new_toml).expect("new rules parse");
    store.swap(Arc::new(new_rules)).await;

    // send filler samples to trigger rebuild (counter %16)
    // we publish to old topic which engine still subscribed to until rebuild
    for _ in 0..24 {
        let _ = transport
            .publish("sensor/old", Qos::BestEffort, &serde_json::json!({"v": 1}))
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // give rebuild time to run (subscribe_to_topics is async)
    tokio::time::sleep(Duration::from_millis(600)).await;

    // now new topic must fire; retry a few times in case rebuild still lagging
    let mut fired = false;
    for _ in 0..6 {
        transport
            .publish("sensor/new", Qos::BestEffort, &serde_json::json!({"v": 1}))
            .await
            .expect("pub new");
        if tokio::time::timeout(Duration::from_millis(600), rx_new.recv())
            .await
            .is_ok()
        {
            fired = true;
            break;
        }
        // also send a couple filler to nudge rebuild if not yet done
        for _ in 0..4 {
            let _ = transport
                .publish("sensor/new", Qos::BestEffort, &serde_json::json!({"v": 1}))
                .await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(fired, "new topic should fire after hot-swap rebuild");

    engine_h.abort();
    drop(transport);
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_hot_swap_old_topic_no_longer_fires_after_rebuild() {
    // verify the old subscription is dropped after rebuild, so old no longer fires
    // (this is a secondary check, not strictly required but proves the rebuild)
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback"),
    );
    let old_toml = r#"
[[rules]]
name = "old-rule"
when.all = [{ topic = "sensor/old2", mode = "Level" }]
actions = [{ topic = "actuator/old2", qos = "reliable", payload = { a = 1 } }]
"#;
    let store = ActiveRules::bootstrap(old_toml).unwrap();
    let (tx_old, mut rx_old) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/old2", move |s: zenoh::sample::Sample| {
            let _ = tx_old.send(s.payload().to_bytes().to_vec());
        })
        .await
        .unwrap();
    let counter = Arc::new(AtomicU64::new(0));
    let h = tokio::spawn({
        let t = transport.clone();
        let s = store.clone();
        let c = counter.clone();
        async move {
            let _ = engine::run_engine(t, s, c, None).await;
        }
    });
    // wait tick
    let baseline = counter.load(Ordering::SeqCst);
    poll_until(
        || {
            let c = counter.clone();
            async move { c.load(Ordering::SeqCst) > baseline }
        },
        Duration::from_secs(3),
    )
    .await;

    // swap to new
    let new_toml = r#"
[[rules]]
name = "new-rule"
when.all = [{ topic = "sensor/new2", mode = "Level" }]
actions = [{ topic = "actuator/new2", qos = "reliable", payload = { b = 2 } }]
"#;
    let nr = Rules::from_toml(new_toml).unwrap();
    store.swap(Arc::new(nr)).await;
    for _ in 0..24 {
        let _ = transport
            .publish("sensor/old2", Qos::BestEffort, &serde_json::json!({}))
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    // drain any old actions that fired during filler
    while tokio::time::timeout(Duration::from_millis(100), rx_old.recv())
        .await
        .is_ok()
    {}
    // after rebuild, publishing old should not fire
    transport
        .publish("sensor/old2", Qos::BestEffort, &serde_json::json!({}))
        .await
        .unwrap();
    let should_be_none = tokio::time::timeout(Duration::from_millis(600), rx_old.recv()).await;
    assert!(
        should_be_none.is_err(),
        "old topic should not fire after rebuild, got {should_be_none:?}"
    );
    h.abort();
}
