use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use serde_json::Value;
use tracing::{debug, info, warn};

use crate::config::RuleStore;
use crate::rules::{Action, Trigger, When};
use crate::transport::Transport;

/// Evaluate a predicate string against a JSON payload.
/// Supports `field == value`, `field != value`, `field < value`, `field > value`,
/// `field <= value`, `field >= value` with string/number/boolean right-hand sides.
/// An empty/absent predicate is always true. Unparseable predicates log and pass
/// (fail-open) so a pure key-expr match still fires.
fn eval_predicate(pred: &Option<String>, payload: &Value) -> bool {
    let Some(pred) = pred else { return true };
    let pred = pred.trim();
    if pred.is_empty() {
        return true;
    }
    let Some((field, op, rhs)) = split_predicate(pred) else {
        warn!(predicate = %pred, "unparseable predicate; treating as true");
        return true;
    };
    let Some(actual) = payload.get(field) else {
        return false;
    };
    let rhs_val: Value = match serde_json::from_str::<Value>(rhs.trim()) {
        Ok(v) => v,
        Err(_) => Value::String(rhs.trim().trim_matches('"').to_string()),
    };
    match op {
        "==" => actual == &rhs_val,
        "!=" => actual != &rhs_val,
        "<" => cmp(actual, &rhs_val).map(|o| o.is_lt()).unwrap_or(false),
        ">" => cmp(actual, &rhs_val).map(|o| o.is_gt()).unwrap_or(false),
        "<=" => cmp(actual, &rhs_val).map(|o| o.is_le()).unwrap_or(false),
        ">=" => cmp(actual, &rhs_val).map(|o| o.is_ge()).unwrap_or(false),
        _ => {
            warn!(op = %op, "unsupported predicate operator; treating as true");
            true
        }
    }
}

fn split_predicate(pred: &str) -> Option<(&str, &str, &str)> {
    for op in ["<=", ">=", "==", "!=", "<", ">"] {
        if let Some((l, r)) = pred.split_once(op) {
            return Some((l.trim(), op, r.trim()));
        }
    }
    None
}

fn cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                xf.partial_cmp(&yf)
            } else {
                None
            }
        }
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// Evaluate one trigger against a single received (topic, payload) sample.
fn trigger_matches(trigger: &Trigger, topic: &str, payload: &Value) -> bool {
    topic == trigger.topic && eval_predicate(&trigger.pred, payload)
}

/// Evaluate a rule's `when` guard over the latest sample for each trigger topic.
/// We hold the most recent payload per topic; a `when` is satisfied when its
/// triggers are satisfied by currently-held samples.
fn when_satisfied(when: &When, latest: &HashMap<String, Value>) -> bool {
    let all_ok = when.all.iter().all(|t| {
        latest
            .get(&t.topic)
            .map(|p| trigger_matches(t, &t.topic, p))
            .unwrap_or(false)
    });
    let any_ok = if when.any.is_empty() {
        true
    } else {
        when.any.iter().any(|t| {
            latest
                .get(&t.topic)
                .map(|p| trigger_matches(t, &t.topic, p))
                .unwrap_or(false)
        })
    };
    all_ok && any_ok
}

/// Run the rule engine: subscribe to sensor topics, maintain latest samples, and
/// fire actions for satisfied rules. One subscription per distinct trigger topic.
pub async fn run_engine(
    transport: Arc<Transport>,
    store: RuleStore,
    eval_counter: Arc<AtomicU64>,
) -> zenoh::Result<()> {
    let rules = store.current().await;
    let mut topics: Vec<String> = Vec::new();
    for rule in &rules.rules {
        collect_topics(&rule.when, &mut topics);
    }
    topics.sort();
    topics.dedup();

    // Open one subscriber per distinct sensor topic; each pushes (topic, payload)
    // into the engine's mpsc channel via a callback running on Zenoh's runtime.
    // Subscriptions are kept alive by Zenoh until the session closes.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, Value)>(256);
    for topic in &topics {
        let tx = tx.clone();
        let topic_key = topic.clone();
        let topic_for_closure = topic_key.clone();
        transport
            .subscribe(&topic_key, move |sample: zenoh::sample::Sample| {
                let payload: Value =
                    serde_json::from_slice(&sample.payload().to_bytes()).unwrap_or(Value::Null);
                let _ = tx.try_send((topic_for_closure.clone(), payload));
            })
            .await?;
    }
    info!(sensor_topics = ?topics, "rule engine subscribed");

    // Latest sample per topic, plus a re-evaluation tick so `when` holds compose.
    let latest: HashMap<String, Value> = HashMap::new();
    let latest = Arc::new(tokio::sync::Mutex::new(latest));

    // Re-evaluation timer so compound `when` fires once all triggers have arrived.
    let eval_latest = latest.clone();
    let eval_store = store.clone();
    let eval_transport = transport.clone();
    let eval_counter = eval_counter.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
        loop {
            tick.tick().await;
            eval_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let snap = eval_latest.lock().await.clone();
            let rules = eval_store.current().await;
            for rule in &rules.rules {
                if when_satisfied(&rule.when, &snap) {
                    info!(rule = %rule.name, "▶ rule fired");
                    for action in &rule.actions {
                        info!(
                            rule = %rule.name,
                            action = %action.topic,
                            qos = ?action.qos,
                            payload = %action.payload,
                            "▶ published action"
                        );
                        fire_action(&eval_transport, action).await;
                    }
                }
            }
        }
    });

    while let Some((topic, payload)) = rx.recv().await {
        latest.lock().await.insert(topic, payload);
    }
    Ok(())
}

fn collect_topics(when: &When, out: &mut Vec<String>) {
    for t in &when.all {
        out.push(t.topic.clone());
    }
    for t in &when.any {
        out.push(t.topic.clone());
    }
}

async fn fire_action(transport: &Transport, action: &Action) {
    if let Err(e) = transport
        .publish(&action.topic, action.qos, &action.payload)
        .await
    {
        warn!(action = %action.topic, error = %e, "action publish failed");
    } else {
        debug!(action = %action.topic, qos = ?action.qos, "fired action");
    }
}
