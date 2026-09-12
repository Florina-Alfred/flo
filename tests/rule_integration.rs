use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use flo_rs::config::ActiveRules;
use flo_rs::engine;
use flo_rs::rules::Qos;
use flo_rs::semantic::{compile, parse_semantic, validate};
use flo_rs::topic::{Pattern, Topic};
use flo_rs::transport::{Envelope, Transport};

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
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );

    let doc = parse_semantic(SEMANTIC_HRC).expect("parse semantic doc");
    validate(&doc).expect("validate");
    let rules = compile(&doc, "7").expect("compile");

    let store = ActiveRules::new(Arc::new(rules));

    let pattern = Pattern::try_new("robot/7/local/drive").unwrap();
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

    let topic = Topic::try_new("robot/7/local/human_present").unwrap();
    transport
        .publish(
            topic,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"separation_distance": 0.5}),
            },
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
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );

    let doc = parse_semantic(SEMANTIC_HRC).expect("parse semantic doc");
    validate(&doc).expect("validate");
    let rules = compile(&doc, "42").expect("compile with robot 42");

    let store = ActiveRules::new(Arc::new(rules));

    let pattern = Pattern::try_new("robot/42/local/drive").unwrap();
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

    let topic = Topic::try_new("robot/42/local/human_present").unwrap();
    transport
        .publish(
            topic,
            Envelope::Action {
                qos: Qos::BestEffort,
                payload: serde_json::json!({"separation_distance": 0.5}),
            },
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
