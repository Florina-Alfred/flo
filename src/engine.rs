use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use serde_json::Value;
use tracing::{debug, info, warn};

use crate::config::RuleStore;
use crate::rules::{Action, EvalMode, Op, Operand, Predicate, PrimitiveRef, Rules, Trigger, When};
use crate::transport::{ManagedSubscriber, Transport};

/// Epsilon for float equality so `==`/`!=` do not fail on IEEE rounding dust.
const EPSILON: f64 = 1e-9;

/// Tracks which robots are in which zones, fed by `zone/*/entered` and
/// `zone/*/cleared` subscriptions. Threaded through the eval tree so
/// `Op::SameZoneAs` can check cross-robot zone overlap.
#[derive(Clone)]
struct ZoneTracker {
    robot_zones: HashMap<String, HashSet<String>>,
}

impl ZoneTracker {
    fn new() -> Self {
        Self {
            robot_zones: HashMap::new(),
        }
    }

    fn enter_zone(&mut self, robot_id: &str, zone_id: &str) {
        self.robot_zones
            .entry(robot_id.to_string())
            .or_default()
            .insert(zone_id.to_string());
    }

    fn clear_zone(&mut self, robot_id: &str, zone_id: &str) {
        if let Some(zones) = self.robot_zones.get_mut(robot_id) {
            zones.remove(zone_id);
            if zones.is_empty() {
                self.robot_zones.remove(robot_id);
            }
        }
    }

    fn share_zone(&self, robot_a: &str, robot_b: &str) -> bool {
        let Some(a_zones) = self.robot_zones.get(robot_a) else {
            return false;
        };
        let Some(b_zones) = self.robot_zones.get(robot_b) else {
            return false;
        };
        a_zones.iter().any(|z| b_zones.contains(z))
    }
}

/// Evaluate a typed predicate against a JSON payload (PRD §C).
/// `None` => no predicate, pure key-expr match, always true (legacy behaviour).
fn eval_predicate(pred: &Option<Predicate>, payload: &Value, zones: &ZoneTracker) -> bool {
    match pred {
        None => true,
        Some(p) => eval_tree(p, payload, zones),
    }
}

/// Recursively walk the typed `Predicate` tree, failing closed on any
/// unsupported node. Unsupported operators or absent payload fields yield
/// `false` rather than fail-open.
fn eval_tree(pred: &Predicate, payload: &Value, zones: &ZoneTracker) -> bool {
    match pred {
        Predicate::Comparison { op, lhs, rhs } => {
            let (Some(l), Some(r)) = (resolve_operand(lhs, payload), resolve_operand(rhs, payload))
            else {
                return false;
            };
            eval_comparison(*op, &l, &r, zones)
        }
        Predicate::And(v) => v.iter().all(|p| eval_tree(p, payload, zones)),
        Predicate::Or(v) => v.iter().any(|p| eval_tree(p, payload, zones)),
        Predicate::Not(b) => !eval_tree(b, payload, zones),
    }
}

/// Compare two resolved JSON values under `op`. Floats use epsilon equality
/// for `==`/`!=`; ordering uses the shared `json_cmp` helper (numbers/strings/bools).
fn eval_comparison(op: Op, l: &Value, r: &Value, zones: &ZoneTracker) -> bool {
    match op {
        Op::Eq => values_equal(l, r),
        Op::Ne => !values_equal(l, r),
        Op::Lt => json_cmp(l, r).is_some_and(|o| o.is_lt()),
        Op::Gt => json_cmp(l, r).is_some_and(|o| o.is_gt()),
        Op::Le => json_cmp(l, r).is_some_and(|o| o.is_le()),
        Op::Ge => json_cmp(l, r).is_some_and(|o| o.is_ge()),
        Op::SameZoneAs => {
            let (Some(a), Some(b)) = (l.as_str(), r.as_str()) else {
                return false;
            };
            zones.share_zone(a, b)
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

fn json_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
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
fn trigger_matches(trigger: &Trigger, topic: &str, payload: &Value, zones: &ZoneTracker) -> bool {
    topic == trigger.topic && eval_predicate(&trigger.pred, payload, zones)
}

/// Evaluate one trigger with edge/level semantics.
/// For Level triggers, returns whether the trigger matches current payload.
/// For Edge triggers, returns whether the outcome CHANGED from the previous tick
/// (false→true = entry fire; true→false = exit fire; first tick never fires).
fn trigger_edge_matches(
    trigger: &Trigger,
    latest: &HashMap<String, Value>,
    prev: &mut HashMap<(String, usize, usize), bool>,
    rule_idx: usize,
    trigger_idx: usize,
    zones: &ZoneTracker,
) -> bool {
    let cur = latest
        .get(&trigger.topic)
        .map(|p| trigger_matches(trigger, &trigger.topic, p, zones))
        .unwrap_or(false);
    match trigger.mode {
        EvalMode::Level => cur,
        EvalMode::Edge => {
            let key = (trigger.topic.clone(), rule_idx, trigger_idx);
            let prev_val = prev.get(&key).copied();
            prev.insert(key, cur);
            prev_val.is_some_and(|p| p != cur)
        }
    }
}

/// Evaluate a `When` guard with per-trigger edge/level transition tracking.
/// `prev` persists across ticks so Edge triggers can detect transitions.
/// Level triggers use current payload match (re-evaluate each tick).
fn when_satisfied_with_prev(
    when: &When,
    latest: &HashMap<String, Value>,
    prev: &mut HashMap<(String, usize, usize), bool>,
    rule_idx: usize,
    zones: &ZoneTracker,
) -> bool {
    let all_ok = when
        .all
        .iter()
        .enumerate()
        .all(|(i, t)| trigger_edge_matches(t, latest, prev, rule_idx, i, zones));
    let any_ok = if when.any.is_empty() {
        true
    } else {
        let offset = when.all.len();
        when.any
            .iter()
            .enumerate()
            .any(|(i, t)| trigger_edge_matches(t, latest, prev, rule_idx, offset + i, zones))
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
    let (sample_tx, mut sample_rx) = tokio::sync::mpsc::channel::<(String, Value)>(256);

    // Zone-tracking: observe zone entered/cleared to support SameZoneAs.
    let zone_tracker = Arc::new(std::sync::Mutex::new(ZoneTracker::new()));
    zone_background(transport.as_ref(), &zone_tracker);

    // Collect initial topics and create subscribers.
    let mut subscribers: Vec<ManagedSubscriber> = Vec::new();
    let mut current_topics: Vec<String> = Vec::new();
    let initial_rules = store.current().await;
    subscribe_to_topics(
        &transport,
        &initial_rules,
        &sample_tx,
        &mut subscribers,
        &mut current_topics,
    )
    .await?;
    info!(sensor_topics = ?current_topics, "rule engine subscribed");

    // Latest sample per topic, plus a re-evaluation tick so `when` holds compose.
    let latest: HashMap<String, Value> = HashMap::new();
    let latest = Arc::new(tokio::sync::Mutex::new(latest));

    // Re-evaluation timer so compound `when` fires once all triggers have arrived.
    let eval_latest = latest.clone();
    let eval_store = store.clone();
    let eval_transport = transport.clone();
    let eval_counter = eval_counter.clone();
    let eval_zones = zone_tracker.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
        let mut prev_outcomes: HashMap<(String, usize, usize), bool> = HashMap::new();
        let mut last_rules: Option<Arc<Rules>> = None;
        loop {
            tick.tick().await;
            eval_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let snap = eval_latest.lock().await.clone();
            let rules = eval_store.current().await;
            let zones = eval_zones.lock().unwrap().clone();

            if last_rules
                .as_ref()
                .is_none_or(|prev| Arc::as_ptr(prev) != Arc::as_ptr(&rules))
            {
                prev_outcomes.clear();
                last_rules = Some(rules.clone());
            }

            for (rule_idx, rule) in rules.rules.iter().enumerate() {
                if when_satisfied_with_prev(&rule.when, &snap, &mut prev_outcomes, rule_idx, &zones)
                {
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

    // Process samples and detect topic changes.
    while let Some((topic, payload)) = sample_rx.recv().await {
        latest.lock().await.insert(topic, payload);
        // Periodically (every 1024 samples) check for topic changes due to hot-swap.
        // This is cheaper than a per-event lock on the store.
        if sample_rx.len() % 256 == 0 {
            let rules = store.current().await;
            let mut new_topics = Vec::new();
            for rule in &rules.rules {
                collect_topics(&rule.when, &mut new_topics);
            }
            new_topics.sort();
            new_topics.dedup();
            if new_topics != current_topics {
                info!("sensor topics changed — rebuilding subscribers");
                let old = std::mem::take(&mut subscribers);
                // Dropping the old Vec drops all subscriber handles, unsubscribing.
                drop(old);
                subscribe_to_topics(
                    &transport,
                    &rules,
                    &sample_tx,
                    &mut subscribers,
                    &mut current_topics,
                )
                .await?;
                info!(sensor_topics = ?current_topics, "subscribers rebuilt");
            }
        }
    }
    Ok(())
}

fn zone_background(transport: &Transport, zone_tracker: &Arc<std::sync::Mutex<ZoneTracker>>) {
    let zt = zone_tracker.clone();
    tokio::spawn({
        let entered = "zone/*/entered";
        let tr = transport.session.clone();
        async move {
            if let Err(e) = tr
                .declare_subscriber(entered)
                .callback(move |sample: zenoh::sample::Sample| {
                    let key = sample.key_expr().to_string();
                    let parts: Vec<&str> = key.split('/').collect();
                    if parts.len() >= 3 {
                        let zone_id = parts[1];
                        let payload: Value = serde_json::from_slice(&sample.payload().to_bytes())
                            .unwrap_or(Value::Null);
                        if let Some(robot_id) = payload.get("robot_id").and_then(|v| v.as_str()) {
                            zt.lock().unwrap().enter_zone(robot_id, zone_id);
                        }
                    }
                })
                .background()
                .await
            {
                warn!(error = %e, topic = entered, "zone entered subscribe failed");
            }
        }
    });
    let zt = zone_tracker.clone();
    tokio::spawn({
        let cleared = "zone/*/cleared";
        let tr = transport.session.clone();
        async move {
            if let Err(e) = tr
                .declare_subscriber(cleared)
                .callback(move |sample: zenoh::sample::Sample| {
                    let key = sample.key_expr().to_string();
                    let parts: Vec<&str> = key.split('/').collect();
                    if parts.len() >= 3 {
                        let zone_id = parts[1];
                        let payload: Value = serde_json::from_slice(&sample.payload().to_bytes())
                            .unwrap_or(Value::Null);
                        if let Some(robot_id) = payload.get("robot_id").and_then(|v| v.as_str()) {
                            zt.lock().unwrap().clear_zone(robot_id, zone_id);
                        }
                    }
                })
                .background()
                .await
            {
                warn!(error = %e, topic = cleared, "zone cleared subscribe failed");
            }
        }
    });
}

/// Subscribe to all distinct topics from the ruleset using managed subscribers.
async fn subscribe_to_topics(
    transport: &Transport,
    rules: &Rules,
    tx: &tokio::sync::mpsc::Sender<(String, Value)>,
    subscribers: &mut Vec<ManagedSubscriber>,
    topics: &mut Vec<String>,
) -> zenoh::Result<()> {
    let mut new_topics: Vec<String> = Vec::new();
    for rule in &rules.rules {
        collect_topics(&rule.when, &mut new_topics);
    }
    new_topics.sort();
    new_topics.dedup();
    for topic in &new_topics {
        let tx = tx.clone();
        let key_expr = topic.clone();
        let sub = transport
            .subscribe_managed(&key_expr, {
                let key = key_expr.clone();
                move |sample: zenoh::sample::Sample| {
                    let payload: Value =
                        serde_json::from_slice(&sample.payload().to_bytes()).unwrap_or(Value::Null);
                    let _ = tx.try_send((key.clone(), payload));
                }
            })
            .await?;
        subscribers.push(sub);
    }
    *topics = new_topics;
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

    fn no_zones() -> ZoneTracker {
        ZoneTracker::new()
    }

    #[test]
    fn none_predicate_always_true() {
        let zones = no_zones();
        assert!(eval_predicate(&None, &json!({}), &zones));
        assert!(eval_predicate(&None, &json!({"anything": 1}), &zones));
    }

    #[test]
    fn comparison_eq_zone_resolves_payload() {
        let zones = no_zones();
        let p = zone_eq("zone_1");
        assert!(eval_tree(&p, &json!({"zone_id": "zone_1"}), &zones));
        assert!(!eval_tree(&p, &json!({"zone_id": "zone_2"}), &zones));
    }

    #[test]
    fn comparison_lt_separation_distance() {
        let zones = no_zones();
        let p = sep_lt(1.2);
        assert!(eval_tree(&p, &json!({"separation_distance": 1.0}), &zones));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.2}), &zones));
        assert!(!eval_tree(&p, &json!({"separation_distance": 2.0}), &zones));
    }

    #[test]
    fn proximity_uses_separation_distance_field() {
        let zones = no_zones();
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::Proximity("human".to_string())),
            rhs: Operand::Float(1.2),
        };
        assert!(eval_tree(&p, &json!({"separation_distance": 0.5}), &zones));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.5}), &zones));
    }

    #[test]
    fn and_all_true_or_any_true_not_negates() {
        let zones = no_zones();
        let and = Predicate::And(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(eval_tree(
            &and,
            &json!({"zone_id": "zone_1", "separation_distance": 1.0}),
            &zones
        ));
        assert!(!eval_tree(
            &and,
            &json!({"zone_id": "zone_1", "separation_distance": 2.0}),
            &zones
        ));

        let or = Predicate::Or(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(eval_tree(
            &or,
            &json!({"zone_id": "zone_2", "separation_distance": 0.5}),
            &zones
        ));
        assert!(!eval_tree(
            &or,
            &json!({"zone_id": "zone_2", "separation_distance": 2.0}),
            &zones
        ));

        let not = Predicate::Not(Box::new(zone_eq("zone_1")));
        assert!(!eval_tree(&not, &json!({"zone_id": "zone_1"}), &zones));
        assert!(eval_tree(&not, &json!({"zone_id": "zone_2"}), &zones));
    }

    #[test]
    fn absent_field_fails_closed() {
        let zones = no_zones();
        // `zone_id` absent => Prim(Zone) resolves to None => false.
        assert!(!eval_tree(&zone_eq("zone_1"), &json!({"other": 1}), &zones));
        assert!(!eval_tree(&sep_lt(1.2), &json!({}), &zones));
        // And with one absent field => whole And false.
        let and = Predicate::And(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(!eval_tree(&and, &json!({"zone_id": "zone_1"}), &zones));
    }

    #[test]
    fn float_equality_uses_epsilon() {
        let zones = no_zones();
        let p = Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::HumanPresence),
            rhs: Operand::Float(1.2),
        };
        // 1.2 vs 1.2000000005 differ by 5e-10 < EPSILON (1e-9) => equal.
        assert!(eval_tree(
            &p,
            &json!({"separation_distance": 1.2000000005}),
            &zones
        ));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.3}), &zones));
    }

    #[test]
    fn same_zone_check_uses_tracker() {
        let mut zones = ZoneTracker::new();

        // No zone data yet — fails closed.
        let p = Predicate::Comparison {
            op: Op::SameZoneAs,
            lhs: Operand::Str("robot7".to_string()),
            rhs: Operand::Str("robot8".to_string()),
        };
        assert!(!eval_tree(&p, &json!({}), &zones));

        // robot7 enters zone_a, robot8 enters zone_a — same zone.
        zones.enter_zone("robot7", "zone_a");
        zones.enter_zone("robot8", "zone_a");
        assert!(eval_tree(&p, &json!({}), &zones));

        // robot8 clears zone_a — no longer same.
        zones.clear_zone("robot8", "zone_a");
        assert!(!eval_tree(&p, &json!({}), &zones));

        // robot8 enters zone_b, robot7 still in zone_a — different.
        zones.enter_zone("robot8", "zone_b");
        assert!(!eval_tree(&p, &json!({}), &zones));

        // Both share zone_c — overlap detected even with different primary zones.
        zones.enter_zone("robot7", "zone_c");
        zones.enter_zone("robot8", "zone_c");
        assert!(eval_tree(&p, &json!({}), &zones));
    }

    #[test]
    fn same_zone_non_string_fails_closed() {
        let zones = no_zones();
        // Operands that don't resolve to strings (e.g. ints) can't be SameZoneAs.
        let p = Predicate::Comparison {
            op: Op::SameZoneAs,
            lhs: Operand::Int(1),
            rhs: Operand::Int(2),
        };
        assert!(!eval_tree(&p, &json!({}), &zones));
    }

    #[test]
    fn zone_tracker_enter_clear_share() {
        let mut z = ZoneTracker::new();

        // Empty tracker — no sharing.
        assert!(!z.share_zone("a", "b"));

        // One robot in a zone — no sharing yet.
        z.enter_zone("a", "z1");
        assert!(!z.share_zone("a", "b"));
        assert!(!z.share_zone("b", "a"));

        // Second robot enters the same zone — share detected.
        z.enter_zone("b", "z1");
        assert!(z.share_zone("a", "b"));
        assert!(z.share_zone("b", "a"));

        // Clear zone — no longer shared.
        z.clear_zone("b", "z1");
        assert!(!z.share_zone("a", "b"));

        // Clear removes empty entry.
        z.clear_zone("a", "z1");
        assert!(!z.share_zone("a", "b"));

        // Unknown robot fails closed.
        assert!(!z.share_zone("a", "c"));
    }

    #[test]
    fn level_trigger_fires_each_tick_while_true() {
        let zones = no_zones();
        let mut prev = HashMap::new();
        let trigger = Trigger {
            topic: "robot/7/proximity".into(),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity("7".into())),
                rhs: Operand::Float(1.2),
            }),
            mode: EvalMode::Level,
        };
        let mut latest = HashMap::new();
        latest.insert(
            "robot/7/proximity".into(),
            json!({"separation_distance": 0.5}),
        );
        let w = When {
            all: vec![trigger],
            any: vec![],
        };

        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
    }

    #[test]
    fn edge_fires_only_on_transition() {
        let zones = no_zones();
        let mut prev = HashMap::new();
        let trigger = Trigger {
            topic: "robot/7/zone".into(),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str("zone_1".into()),
            }),
            mode: EvalMode::Edge,
        };
        let w = When {
            all: vec![trigger],
            any: vec![],
        };

        let outside = json!({"zone_id": "zone_2"});
        let inside = json!({"zone_id": "zone_1"});

        // Tick 1: outside — no baseline, no fire
        let mut latest = HashMap::new();
        latest.insert("robot/7/zone".into(), outside.clone());
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));

        // Tick 2: enter — false→true, fire
        latest.insert("robot/7/zone".into(), inside.clone());
        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));

        // Tick 3: hold — true→true, no fire
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));

        // Tick 4: exit — true→false, fire
        latest.insert("robot/7/zone".into(), outside.clone());
        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));

        // Tick 5: still absent — false→false, no fire
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
    }
}
