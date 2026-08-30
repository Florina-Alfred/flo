use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::rules::Qos;
use flo_rs::transport::Transport;

// INFRA-09: flaky-sleep hardening — the engine's subscription readiness is
// gated via `engine::subscribed` oneshot (like `runtime::await_engine_ready`
// does) where feasible, and eval_counter polling uses a deadline-based retry
// with bounded timeout (not infinite sleep) so CI load doesn't flap. The
// pattern is: wait for readiness via oneshot, then poll counter with
// deadline (10s) and short 10ms interval — fast when uncontended, robust
// when loaded. Timeouts for action delivery are also increased to 10s.

async fn wait_for_counter(counter: &AtomicU64, target: u64, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if counter.load(Ordering::SeqCst) >= target {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for eval_counter >= {target} (current {})",
                counter.load(Ordering::SeqCst)
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sensor_sample_triggers_action() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );

    let store = RuleStore::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "trigger-on-data""#,
        "\nwhen.all = [{ topic = \"sensor/foo\", mode = \"Level\" }]\n",
        r#"actions = [{ topic = "actuator/bar", qos = "reliable", payload = { triggered = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/bar", move |s: zenoh::sample::Sample| {
            let _ = tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe action topic");

    let eval_counter = Arc::new(AtomicU64::new(0));
    let eval_counter_for_engine = eval_counter.clone();
    let engine_transport = transport.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let engine = tokio::spawn(async move {
        engine::run_engine(
            engine_transport,
            store,
            eval_counter_for_engine,
            Some(ready_tx),
        )
        .await
        .expect("engine run");
    });

    // Gate on the engine's subscription oneshot — robust under load.
    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("engine should confirm subscriptions within 5s")
        .expect("subscribed signal");

    transport
        .publish(
            "sensor/foo",
            Qos::BestEffort,
            &serde_json::json!({"value": 42}),
        )
        .await
        .expect("publish sensor sample");

    // Allow at least two ticks for the sample to be processed and action
    // to be published — deadline-based, not flaky 20ms loop.
    let after_pub = eval_counter.load(Ordering::SeqCst);
    wait_for_counter(&eval_counter, after_pub + 2, Duration::from_secs(10)).await;

    let result = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("timeout waiting for action (10s)")
        .expect("action channel closed");

    let payload: serde_json::Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(payload["triggered"], true);

    drop(transport);
    engine.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_data_no_action() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );

    let store = RuleStore::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "trigger-on-data""#,
        "\nwhen.all = [{ topic = \"sensor/never\", mode = \"Level\" }]\n",
        r#"actions = [{ topic = "actuator/silent", qos = "reliable", payload = { triggered = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/silent", move |s: zenoh::sample::Sample| {
            let _ = tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe action topic");

    let eval_counter = Arc::new(AtomicU64::new(0));
    let eval_counter_for_engine = eval_counter.clone();
    let engine_transport = transport.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let engine = tokio::spawn(async move {
        engine::run_engine(
            engine_transport,
            store,
            eval_counter_for_engine,
            Some(ready_tx),
        )
        .await
        .expect("engine run");
    });

    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("engine ready")
        .expect("subscribed");

    wait_for_counter(&eval_counter, 5, Duration::from_secs(10)).await;

    let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(result.is_err(), "no action should fire without sensor data");

    drop(transport);
    engine.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn zone_path_uses_managed_subscription_lifecycle() {
    // The zone `entered`/`cleared` subscriptions go through `Transport`'s
    // managed subscription lifecycle. This proves the zone tracker is fed by a
    // live managed subscription: a `SameZoneAs` rule fires only once both robots
    // have entered the same zone over that path.
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );

    let store = RuleStore::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "zone-collision""#,
        "\nwhen.all = [{ topic = \"sensor/probe\", pred = { Comparison = { op = \"SameZoneAs\", lhs = { Str = \"robot-a\" }, rhs = { Str = \"robot-b\" } } } }]\n",
        r#"actions = [{ topic = "actuator/warn", qos = "reliable", payload = { colliding = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("actuator/warn", move |s: zenoh::sample::Sample| {
            let _ = tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe action topic");

    let eval_counter = Arc::new(AtomicU64::new(0));
    let engine_counter = eval_counter.clone();
    let engine_transport = transport.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, engine_counter, Some(ready_tx))
            .await
            .expect("engine run");
    });

    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("engine ready")
        .expect("subscribed");

    // Only one robot in the zone: no collision yet.
    transport
        .publish(
            "zone/cell-3/entered",
            Qos::BestEffort,
            &serde_json::json!({"robot_id": "robot-a"}),
        )
        .await
        .expect("publish zone entered");
    transport
        .publish(
            "sensor/probe",
            Qos::BestEffort,
            &serde_json::json!({"value": 1}),
        )
        .await
        .expect("publish probe");
    let no_action = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(no_action.is_err(), "single-zone robot must not collide");

    // Second robot enters the same zone -> SameZoneAs now holds.
    transport
        .publish(
            "zone/cell-3/entered",
            Qos::BestEffort,
            &serde_json::json!({"robot_id": "robot-b"}),
        )
        .await
        .expect("publish second zone entered");
    transport
        .publish(
            "sensor/probe",
            Qos::BestEffort,
            &serde_json::json!({"value": 1}),
        )
        .await
        .expect("publish probe again");

    let result = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("timeout waiting for action (10s)")
        .expect("action channel closed");
    let payload: serde_json::Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(payload["colliding"], true);

    drop(transport);
    engine.abort();
}
