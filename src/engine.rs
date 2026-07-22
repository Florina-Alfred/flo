use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use serde_json::Value;
use tracing::{debug, info, warn};

use crate::config::RuleStore;
use crate::rules::{Action, Op, Operand, Predicate, PrimitiveRef, Trigger, When};
use crate::transport::Transport;

/// Epsilon for float equality so `==`/`!=` do not fail on IEEE rounding dust.
const EPSILON: f64 = 1e-9;

/// Evaluate a typed predicate against a JSON payload (PRD §C).
/// `None` => no predicate, pure key-expr match, always true (legacy behaviour).
fn eval_predicate(pred: &Option<Predicate>, payload: &Value) -> bool {
    match pred {
        None => true,
        Some(p) => eval_tree(p, payload),
    }
}

/// Recursively walk the typed `Predicate` tree, failing closed on any
/// unsupported node. Unsupported operators or absent payload fields yield
/// `false` rather than fail-open.
fn eval_tree(pred: &Predicate, payload: &Value) -> bool {
    match pred {
        Predicate::Comparison { op, lhs, rhs } => {
            let (Some(l), Some(r)) = (resolve_operand(lhs, payload), resolve_operand(rhs, payload))
            else {
                return false;
            };
            eval_comparison(*op, &l, &r)
        }
        Predicate::And(v) => v.iter().all(|p| eval_tree(p, payload)),
        Predicate::Or(v) => v.iter().any(|p| eval_tree(p, payload)),
        Predicate::Not(b) => !eval_tree(b, payload),
    }
}

/// Compare two resolved JSON values under `op`. Floats use epsilon equality
/// for `==`/`!=`; ordering uses the shared `cmp` helper (numbers/strings/bools).
fn eval_comparison(op: Op, l: &Value, r: &Value) -> bool {
    match op {
        Op::Eq => values_equal(l, r),
        Op::Ne => !values_equal(l, r),
        Op::Lt => cmp(l, r).is_some_and(|o| o.is_lt()),
        Op::Gt => cmp(l, r).is_some_and(|o| o.is_gt()),
        Op::Le => cmp(l, r).is_some_and(|o| o.is_le()),
        Op::Ge => cmp(l, r).is_some_and(|o| o.is_ge()),
        Op::SameZoneAs => {
            warn!("Op::SameZoneAs not yet supported; failing closed");
            false
        }
    }
}

/// Equality with epsilon tolerance for floats, exact match otherwise.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                (xf - yf).abs() < EPSILON
            } else {
                false
            }
        }
        _ => a == b,
    }
}

/// Resolve an `Operand` to a JSON value drawn from the payload.
/// `None` means the referenced field is absent and cannot satisfy the predicate.
fn resolve_operand(op: &Operand, payload: &Value) -> Option<Value> {
    match op {
        Operand::Bool(v) => Some(Value::Bool(*v)),
        Operand::Int(v) => Some(Value::Number((*v).into())),
        Operand::Float(v) => Some(serde_json::Number::from_f64(*v).map(Value::Number)?),
        Operand::Str(v) => Some(Value::String(v.clone())),
        Operand::Prim(p) => {
            let field = prim_field(p);
            payload.get(field).cloned()
        }
    }
}

/// Map a `PrimitiveRef` to its JSON payload field name (PRD §4).
fn prim_field(p: &PrimitiveRef) -> &'static str {
    match p {
        PrimitiveRef::Zone => "zone_id",
        PrimitiveRef::Robot => "role",
        PrimitiveRef::HumanPresence => "separation_distance",
        PrimitiveRef::Proximity(_) => "separation_distance",
        PrimitiveRef::Site => "site_id",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zone_eq(z: &str) -> Predicate {
        Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::Zone),
            rhs: Operand::Str(z.to_string()),
        }
    }

    fn sep_lt(d: f64) -> Predicate {
        Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::HumanPresence),
            rhs: Operand::Float(d),
        }
    }

    #[test]
    fn none_predicate_always_true() {
        assert!(eval_predicate(&None, &json!({})));
        assert!(eval_predicate(&None, &json!({"anything": 1})));
    }

    #[test]
    fn comparison_eq_zone_resolves_payload() {
        let p = zone_eq("zone_1");
        assert!(eval_tree(&p, &json!({"zone_id": "zone_1"})));
        assert!(!eval_tree(&p, &json!({"zone_id": "zone_2"})));
    }

    #[test]
    fn comparison_lt_separation_distance() {
        let p = sep_lt(1.2);
        assert!(eval_tree(&p, &json!({"separation_distance": 1.0})));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.2})));
        assert!(!eval_tree(&p, &json!({"separation_distance": 2.0})));
    }

    #[test]
    fn proximity_uses_separation_distance_field() {
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::Proximity("human".to_string())),
            rhs: Operand::Float(1.2),
        };
        assert!(eval_tree(&p, &json!({"separation_distance": 0.5})));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.5})));
    }

    #[test]
    fn and_all_true_or_any_true_not_negates() {
        let and = Predicate::And(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(eval_tree(
            &and,
            &json!({"zone_id": "zone_1", "separation_distance": 1.0})
        ));
        assert!(!eval_tree(
            &and,
            &json!({"zone_id": "zone_1", "separation_distance": 2.0})
        ));

        let or = Predicate::Or(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(eval_tree(
            &or,
            &json!({"zone_id": "zone_2", "separation_distance": 0.5})
        ));
        assert!(!eval_tree(
            &or,
            &json!({"zone_id": "zone_2", "separation_distance": 2.0})
        ));

        let not = Predicate::Not(Box::new(zone_eq("zone_1")));
        assert!(!eval_tree(&not, &json!({"zone_id": "zone_1"})));
        assert!(eval_tree(&not, &json!({"zone_id": "zone_2"})));
    }

    #[test]
    fn absent_field_fails_closed() {
        // `zone_id` absent => Prim(Zone) resolves to None => false.
        assert!(!eval_tree(&zone_eq("zone_1"), &json!({"other": 1})));
        assert!(!eval_tree(&sep_lt(1.2), &json!({})));
        // And with one absent field => whole And false.
        let and = Predicate::And(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(!eval_tree(&and, &json!({"zone_id": "zone_1"})));
    }

    #[test]
    fn float_equality_uses_epsilon() {
        let p = Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::HumanPresence),
            rhs: Operand::Float(1.2),
        };
        // 1.2 vs 1.2000000005 differ by 5e-10 < EPSILON (1e-9) => equal.
        assert!(eval_tree(&p, &json!({"separation_distance": 1.2000000005})));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.3})));
    }

    #[test]
    fn same_zone_as_unsupported_fails_closed() {
        let p = Predicate::Comparison {
            op: Op::SameZoneAs,
            lhs: Operand::Prim(PrimitiveRef::Zone),
            rhs: Operand::Str("zone_1".to_string()),
        };
        assert!(!eval_tree(&p, &json!({"zone_id": "zone_1"})));
    }
}
