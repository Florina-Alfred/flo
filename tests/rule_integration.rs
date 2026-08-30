use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use flo_rs::config::ActiveRules;
use flo_rs::engine;
use flo_rs::rules::Qos;
use flo_rs::semantic::{compile, parse_semantic, validate};
use flo_rs::transport::Transport;

const SEMANTIC_HRC: &str = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "slow-on-proximity"
when.near_human = 1.5
actions = [ { slow_to = 0.2, qos = "best_effort" } ]
"#;

#[tokio::test(flavor = "multi_thread")]
async fn semantic_compile_to_engine_e2e() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );

    let doc = parse_semantic(SEMANTIC_HRC).expect("parse semantic doc");
    validate(&doc).expect("validate");
    let rules = compile(&doc, "7").expect("compile");

    let store = ActiveRules::new(Arc::new(rules));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("robot/7/local/drive", move |s: zenoh::sample::Sample| {
            let _ = tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe action topic");

    let eval_counter = Arc::new(AtomicU64::new(0));
    let engine_counter = eval_counter.clone();
    let engine_transport = transport.clone();
    let engine_handle = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, engine_counter, None)
            .await
            .expect("engine run");
    });

    // Wait for engine to subscribe and start ticking before publishing.
    let baseline = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < baseline + 1 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Publish human_present data to trigger the near_human < 1.5 rule
    transport
        .publish(
            "robot/7/local/human_present",
            Qos::BestEffort,
            &serde_json::json!({"separation_distance": 0.5}),
        )
        .await
        .expect("publish sensor data");

    let after_pub = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < after_pub + 3 {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let result = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for action")
        .expect("action channel closed");

    let payload: serde_json::Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(payload["speed_mps"], 0.2);

    drop(transport);
    engine_handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn compile_with_custom_robot_id_routes_topics() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );

    let doc = parse_semantic(SEMANTIC_HRC).expect("parse semantic doc");
    validate(&doc).expect("validate");
    let rules = compile(&doc, "42").expect("compile with robot 42");

    let store = ActiveRules::new(Arc::new(rules));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    transport
        .subscribe("robot/42/local/drive", move |s: zenoh::sample::Sample| {
            let _ = tx.send(s.payload().to_bytes().to_vec());
        })
        .await
        .expect("subscribe action topic");

    let eval_counter = Arc::new(AtomicU64::new(0));
    let engine_counter = eval_counter.clone();
    let engine_transport = transport.clone();
    let engine_handle = tokio::spawn(async move {
        engine::run_engine(engine_transport, store, engine_counter, None)
            .await
            .expect("engine run");
    });

    let baseline = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < baseline + 1 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    transport
        .publish(
            "robot/42/local/human_present",
            Qos::BestEffort,
            &serde_json::json!({"separation_distance": 0.5}),
        )
        .await
        .expect("publish sensor data");

    let after_pub = eval_counter.load(Ordering::SeqCst);
    while eval_counter.load(Ordering::SeqCst) < after_pub + 3 {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let result = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for action")
        .expect("action channel closed");

    let payload: serde_json::Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(payload["speed_mps"], 0.2);

    drop(transport);
    engine_handle.abort();
}
