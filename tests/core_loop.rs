mod helpers;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use flo_rs::config::ActiveRules;
use flo_rs::engine;
use flo_rs::health::ReadyGate;
use flo_rs::rules::Qos;
use flo_rs::topic::{Pattern, Topic};
use flo_rs::transport::{Envelope, Transport};
use helpers::wait_for_counter;

async fn wait_for_ready(gate: &ReadyGate, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if gate.is_ready_public() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for ReadyGate");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sensor_sample_triggers_action() {
    let transport = Arc::new(
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );

    let store = ActiveRules::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "trigger-on-data""#,
        "\nwhen.all = [{ topic = \"sensor/foo\", mode = \"Level\" }]\n",
        r#"actions = [{ topic = "actuator/bar", qos = "reliable", payload = { triggered = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let pattern = Pattern::try_new("actuator/bar").unwrap();
    let sub = transport
        .subscribe(pattern)
        .await
        .expect("subscribe action topic");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let _ = tx.send(sample.payload().to_bytes().to_vec());
        }
    });

    let gate = ReadyGate::new();
    let eval_counter = gate.eval_counter();
    let engine_transport = transport.clone();
    let gate_clone = gate.clone();
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, gate_clone)
            .await
            .expect("engine run");
    });

    wait_for_ready(&gate, Duration::from_secs(5)).await;

    let topic = Topic::try_new("sensor/foo").unwrap();
    transport
        .publish(
            topic,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"value": 42}),
            },
        )
        .await
        .expect("publish sensor sample");

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
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );

    let store = ActiveRules::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "trigger-on-data""#,
        "\nwhen.all = [{ topic = \"sensor/never\", mode = \"Level\" }]\n",
        r#"actions = [{ topic = "actuator/silent", qos = "reliable", payload = { triggered = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let pattern = Pattern::try_new("actuator/silent").unwrap();
    let sub = transport
        .subscribe(pattern)
        .await
        .expect("subscribe action topic");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let _ = tx.send(sample.payload().to_bytes().to_vec());
        }
    });

    let gate = ReadyGate::new();
    let eval_counter = gate.eval_counter();
    let engine_transport = transport.clone();
    let gate_clone = gate.clone();
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, gate_clone)
            .await
            .expect("engine run");
    });

    wait_for_ready(&gate, Duration::from_secs(5)).await;

    wait_for_counter(&eval_counter, 5, Duration::from_secs(10)).await;

    let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(result.is_err(), "no action should fire without sensor data");

    drop(transport);
    engine.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn zone_path_uses_managed_subscription_lifecycle() {
    let transport = Arc::new(
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );

    let store = ActiveRules::bootstrap(concat!(
        "[[rules]]\n",
        r#"name = "zone-collision""#,
        "\nwhen.all = [{ topic = \"sensor/probe\", pred = { Comparison = { op = \"SameZoneAs\", lhs = { Str = \"robot-a\" }, rhs = { Str = \"robot-b\" } } } }]\n",
        r#"actions = [{ topic = "actuator/warn", qos = "reliable", payload = { colliding = true } }]"#,
        "\n",
    ))
    .expect("bootstrap rules");

    let pattern = Pattern::try_new("actuator/warn").unwrap();
    let sub = transport
        .subscribe(pattern)
        .await
        .expect("subscribe action topic");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let _ = tx.send(sample.payload().to_bytes().to_vec());
        }
    });

    let gate = ReadyGate::new();
    let engine_transport = transport.clone();
    let gate_clone = gate.clone();
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, gate_clone)
            .await
            .expect("engine run");
    });

    wait_for_ready(&gate, Duration::from_secs(5)).await;

    // Only one robot in the zone: no collision yet.
    let t = Topic::try_new("zone/cell-3/entered").unwrap();
    transport
        .publish(
            t,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"robot_id": "robot-a"}),
            },
        )
        .await
        .expect("publish zone entered");
    let t2 = Topic::try_new("sensor/probe").unwrap();
    transport
        .publish(
            t2,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"value": 1}),
            },
        )
        .await
        .expect("publish probe");
    let no_action = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(no_action.is_err(), "single-zone robot must not collide");

    // Second robot enters the same zone -> SameZoneAs now holds.
    let t3 = Topic::try_new("zone/cell-3/entered").unwrap();
    transport
        .publish(
            t3,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"robot_id": "robot-b"}),
            },
        )
        .await
        .expect("publish second zone entered");
    let t4 = Topic::try_new("sensor/probe").unwrap();
    transport
        .publish(
            t4,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"value": 1}),
            },
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
