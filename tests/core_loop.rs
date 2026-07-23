use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::rules::Qos;
use flo_rs::transport::Transport;

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
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, eval_counter_for_engine)
            .await
            .expect("engine run");
    });

    let baseline = eval_counter.load(Ordering::SeqCst);

    // Wait for the engine's re-eval loop to start ticking, proving subscriptions
    // are active and the sample channel is live.
    while eval_counter.load(Ordering::SeqCst) < baseline + 1 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    transport
        .publish(
            "sensor/foo",
            Qos::BestEffort,
            &serde_json::json!({"value": 42}),
        )
        .await
        .expect("publish sensor sample");

    // Allow at least two more ticks for the sample to be processed and action
    // to be published.
    let after_pub = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < after_pub + 2 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let result = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for action")
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
    let engine = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, eval_counter_for_engine)
            .await
            .expect("engine run");
    });

    let baseline = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < baseline + 5 {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let result = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(result.is_err(), "no action should fire without sensor data");

    drop(transport);
    engine.abort();
}
