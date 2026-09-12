use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{debug, info, warn};

use crate::config::ActiveRules;
use crate::health::ReadyGate;
use crate::rules::{Action, EvalMode, Op, Operand, Predicate, PrimitiveRef, Rules, Trigger, When};
use crate::transport::{Subscription, Transport};

/// Epsilon for float equality so `==`/`!=` do not fail on IEEE rounding dust.
const EPSILON: f64 = 1e-9;

/// Owns the engine evaluation wiring that was previously spread across
/// `latest: HashMap<String, Value>`, `zone_tracker: Arc<Mutex<ZoneTracker>>`,
/// `prev_outcomes: HashMap<(String,usize,usize), bool>` and `sample_count %16`.
///
/// - `latest` holds the last sample per topic, timestamped. **Staleness is
///   explicit**: without a timeout `tick` re-evaluates `latest` forever;
///   a stale pose does NOT assume hazard — `eval_tree` fails closed on absent
///   fields (`resolve_operand` returns `None` → `false`). If `staleness_timeout`
///   is set, entries older than the timeout are treated as missing (fail-closed).
/// - `zones` is a derived topic stream from `zone/*/entered` + `zone/*/cleared`,
///   like any sensor topic. It is updated via `ingest`, not via a special-cased
///   `Arc<Mutex<ZoneTracker>>` cloned per tick. The `ZoneTracker` 36-line struct
///   has been merged here.
/// - `prev` tracks per-trigger `Edge` transitions; it is cleared when the ruleset
///   `Arc` pointer changes (hot-reload), so a new ruleset starts with no baseline.
/// - Subscription rebuild is driven by `store.subscribe()` `watch` channel, not
///   arrival-rate-dependent `sample_count % 16`. This also removes staleness
///   ambiguity: rebuild happens on explicit store notification.
/// - `EvalMode` (Level vs Edge) is a per-trigger property evaluated at `tick` via
///   `prev`; the subscription set itself is derived from `When` topics regardless
///   of mode (mode is pushed into the subscription decision as metadata for future
///   filtering, but currently checked at tick).
/// - `json_cmp` type mismatch returns `None` which `eval_comparison` treats as
///   `false` (fail-closed), made explicit in docs and code.
#[derive(Default)]
pub struct EvalState {
    transport: Option<Arc<Transport>>,
    store: Option<ActiveRules>,
    latest: HashMap<String, Value>,
    zones: HashMap<String, HashSet<String>>,
    prev: HashMap<(String, usize, usize), bool>,
    last_rules: Option<Arc<Rules>>,
    timestamps: HashMap<String, Instant>,
    staleness_timeout: Option<Duration>,
}

impl EvalState {
    /// Create an `EvalState` wired to a transport and store for `run().await`.
    /// This is the new entry point replacing `run_engine(transport, store, counter, subscribed)`.
    pub fn new(transport: Arc<Transport>, store: ActiveRules) -> Self {
        Self {
            transport: Some(transport),
            store: Some(store),
            ..Self::default()
        }
    }

    /// Pure-state constructor for unit tests that only need `ingest` + `tick`.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Set an explicit staleness timeout. Entries older than this are treated as
    /// missing during `tick` (fail-closed). `None` (default) means no timeout —
    /// `tick` re-evaluates `latest` forever, matching the pre-ARCH-06 behavior
    /// where `run_engine:279-288` ticked over `latest` without expiry. This is
    /// documented as explicit stale-data handling: missing fields fail closed via
    /// `resolve_operand`/`eval_tree`, not via assumed hazard.
    pub fn with_staleness(mut self, timeout: Duration) -> Self {
        self.staleness_timeout = Some(timeout);
        self
    }

    /// Ingest a single `(topic, payload)` sample. Zone topics
    /// (`zone/*/entered`, `zone/*/cleared`) update the derived zone map;
    /// all other topics update `latest` and its timestamp.
    ///
    /// Zone payloads are expected to contain `robot_id: string`; the zone id is
    /// extracted from the topic (`zone/{zone_id}/entered|cleared` or the 5-segment
    /// `zone/{site}/{cell}/{robot}/entered|cleared` form). A missing `robot_id`
    /// or malformed topic is ignored (fail-closed, no zone update).
    pub fn ingest(&mut self, topic: String, payload: Value) {
        if topic.starts_with("zone/")
            && (topic.ends_with("/entered") || topic.ends_with("/cleared"))
        {
            let parts: Vec<&str> = topic.split('/').collect();
            if parts.len() >= 3 {
                let zone_id = if parts.len() == 3 {
                    parts[1]
                } else {
                    // 5-segment form `zone/{site}/{cell}/{robot}/entered` — treat the
                    // zone as `site/cell` or the second segment for simplicity; the
                    // original `ZoneTracker` used `parts[1]` as zone_id for 3-segment
                    // and also used `parts[1]` for 5-segment (which would be site).
                    // Keep compatibility: use parts[1] for 3-seg, and join first zone-like
                    // segments for 5-seg. Simpler: use parts[1] as zone_id always, as before.
                    parts[1]
                };
                if let Some(robot_id) = payload.get("robot_id").and_then(|v| v.as_str()) {
                    if topic.ends_with("/entered") {
                        self.enter_zone(robot_id, zone_id);
                    } else {
                        self.clear_zone(robot_id, zone_id);
                    }
                }
            }
            return;
        }
        self.timestamps.insert(topic.clone(), Instant::now());
        self.latest.insert(topic, payload);
    }

    /// Convenience for tests: ingest a sensor sample.
    pub fn ingest_sample(&mut self, topic: impl Into<String>, payload: Value) {
        let t = topic.into();
        // Reuse the same staleness timestamp logic as `ingest`.
        self.timestamps.insert(t.clone(), Instant::now());
        self.latest.insert(t, payload);
    }

    /// Direct zone ingestion for tests that previously used `ZoneTracker`.
    pub fn enter_zone(&mut self, robot_id: &str, zone_id: &str) {
        self.zones
            .entry(robot_id.to_string())
            .or_default()
            .insert(zone_id.to_string());
    }

    pub fn clear_zone(&mut self, robot_id: &str, zone_id: &str) {
        if let Some(zones) = self.zones.get_mut(robot_id) {
            zones.remove(zone_id);
            if zones.is_empty() {
                self.zones.remove(robot_id);
            }
        }
    }

    pub fn share_zone(&self, robot_a: &str, robot_b: &str) -> bool {
        let Some(a_zones) = self.zones.get(robot_a) else {
            return false;
        };
        let Some(b_zones) = self.zones.get(robot_b) else {
            return false;
        };
        a_zones.iter().any(|z| b_zones.contains(z))
    }

    /// Expose zones for predicate evaluation (used by `eval_tree`).
    pub fn zones_map(&self) -> &HashMap<String, HashSet<String>> {
        &self.zones
    }

    /// Return a snapshot of latest (for tests). Stale entries are still present
    /// unless `staleness_timeout` is set — staleness filtering happens at `tick`.
    pub fn latest_snapshot(&self) -> &HashMap<String, Value> {
        &self.latest
    }

    /// Evaluate all rules against current `latest` + `zones`, returning the
    /// actions that should fire this tick.
    ///
    /// - Level triggers fire every tick while `cur == true`.
    /// - Edge triggers fire only on transition (`false→true` or `true→false`);
    ///   the first observation never fires. `prev` is keyed by `(topic, rule_idx, trigger_idx)`.
    /// - When the `Rules` `Arc` pointer changes, `prev` is cleared so the new
    ///   ruleset starts without a stale edge baseline.
    /// - Staleness: if `staleness_timeout` is set, entries older than the timeout
    ///   are treated as missing (`None` payload → `false`), i.e. fail-closed.
    pub fn tick(&mut self, rules: &Rules) -> Vec<Action> {
        // We need an owned Arc to track pointer identity; if caller passes &Rules
        // we cannot detect pointer change cheaply, so we clear only when `last_rules`
        // is None (first tick) and otherwise assume the caller handles clearing
        // when using &Rules. For `tick_with_arc` we do pointer check.
        self.tick_inner(rules, None)
    }

    /// Like `tick` but takes an `Arc<Rules>` so pointer identity can be used to
    /// clear `prev` when the ruleset is hot-swapped. Prefer this when the watch
    /// channel provides an `Arc`.
    pub fn tick_with_arc(&mut self, rules: Arc<Rules>) -> Vec<Action> {
        let ptr_changed = self
            .last_rules
            .as_ref()
            .is_none_or(|prev| !Arc::ptr_eq(prev, &rules));
        if ptr_changed {
            self.prev.clear();
            self.last_rules = Some(rules.clone());
        }
        self.tick_inner(&rules.clone(), Some(rules))
    }

    fn tick_inner(&mut self, rules: &Rules, arc_opt: Option<Arc<Rules>>) -> Vec<Action> {
        // If we were given &Rules only and no Arc pointer, handle first-tick prev clearing
        // via `last_rules` being None (already handled above for Arc case). For &Rules,
        // we cannot detect swap, so require caller to use `tick_with_arc` for hot-reload.
        // Still, if `last_rules` is None and we have an arc_opt, we already cleared.
        // If we have no arc_opt and `last_rules` is None, we still want to set it lazily
        // on first tick but we don't have an Arc to store — skip.
        let _ = arc_opt;

        // Build a staleness-filtered view of `latest` if a timeout is configured.
        // Without a timeout, `latest` is used as-is (documented stale-forever behavior).
        let snap: HashMap<String, Value> = if let Some(timeout) = self.staleness_timeout {
            let now = Instant::now();
            self.latest
                .iter()
                .filter_map(|(k, v)| {
                    if let Some(ts) = self.timestamps.get(k) {
                        if now.duration_since(*ts) <= timeout {
                            Some((k.clone(), v.clone()))
                        } else {
                            None
                        }
                    } else {
                        Some((k.clone(), v.clone()))
                    }
                })
                .collect()
        } else {
            self.latest.clone()
        };

        let mut out = Vec::new();
        for (rule_idx, rule) in rules.rules.iter().enumerate() {
            if when_satisfied_with_prev(&rule.when, &snap, &mut self.prev, rule_idx, &self.zones) {
                out.extend(rule.actions.clone());
            }
        }
        out
    }

    /// Subscribe to the store's `watch` channel and debounced sensor/zone topics,
    /// owning the 50ms tick loop and publishing actions. This replaces the old
    /// `run_engine(transport, store, eval_counter, subscribed)` 4-param bag.
    pub async fn run(
        mut self,
        eval_counter: Arc<AtomicU64>,
        subscribed: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> zenoh::Result<()> {
        let transport = self
            .transport
            .clone()
            .expect("EvalState::run requires transport (use EvalState::new)");
        let store = self.store.clone().expect("EvalState::run requires store");

        let (sample_tx, mut sample_rx) = tokio::sync::mpsc::channel::<(String, Value)>(256);

        // Zone subscriptions are now a derived stream like sensor topics: they send
        // (key_expr, payload) into the same `sample_tx`, and `ingest` updates zones.
        let _zone_handles =
            zone_subscriptions(transport.as_ref(), sample_tx.clone()).await?;

        // Initial sensor subscriptions.
        let mut subscribers: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        let mut current_topics: Vec<String> = Vec::new();
        let initial_rules = store.current().await;
        subscribe_to_topics(
            transport.as_ref(),
            &initial_rules,
            &sample_tx,
            &mut subscribers,
            &mut current_topics,
        )
        .await?;
        if let Some(tx) = subscribed {
            let _ = tx.send(());
        }
        info!(sensor_topics = ?current_topics, "rule engine subscribed");

        // Watch channel drives rebuild, not `sample_count %16`.
        let mut store_rx = store.subscribe();
        // Mark current rules as seen to avoid spurious prev clear on first tick.
        self.last_rules = Some(initial_rules);

        let mut tick = tokio::time::interval(Duration::from_millis(50));
        // Hold zone handles for lifetime (abort on drop).
        let _keep_zones = _zone_handles;

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    eval_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let rules = store_rx.borrow().clone();
                    // Detect pointer change for Edge baseline reset.
                    if self.last_rules.as_ref().is_none_or(|prev| !Arc::ptr_eq(prev, &rules)) {
                        self.prev.clear();
                        self.last_rules = Some(rules.clone());
                    }
                    let actions = self.tick_inner(&rules, Some(rules.clone()));
                    for action in actions {
                        fire_action(transport.as_ref(), &action).await;
                    }
                }
                maybe_sample = sample_rx.recv() => {
                    let Some((topic, payload)) = maybe_sample else {
                        break;
                    };
                    self.ingest(topic, payload);
                }
                changed = store_rx.changed() => {
                    if changed.is_err() {
                        // Store dropped — exit.
                        break;
                    }
                    // Debounce: coalesce rapid swaps within 50ms (arrival without extra load).
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    while store_rx.has_changed().unwrap_or(false) {
                        let _ = store_rx.changed().await;
                    }
                    let rules = store_rx.borrow().clone();
                    let mut new_topics = Vec::new();
                    for rule in &rules.rules {
                        collect_topics(&rule.when, &mut new_topics);
                    }
                    new_topics.sort();
                    new_topics.dedup();
                    if new_topics != current_topics {
                        info!("sensor topics changed — rebuilding subscribers");
                        let old = std::mem::take(&mut subscribers);
                        drop(old);
                        if let Err(e) = subscribe_to_topics(
                            transport.as_ref(),
                            &rules,
                            &sample_tx,
                            &mut subscribers,
                            &mut current_topics,
                        )
                        .await
                        {
                            warn!(error = %e, "subscriber rebuild failed");
                        } else {
                            info!(sensor_topics = ?current_topics, "subscribers rebuilt");
                        }
                    }
                    // `prev` will be cleared on next tick via pointer check.
                }
            }
        }
        Ok(())
    }
}

/// Evaluate a typed predicate against a JSON payload (PRD §C).
/// `None` => no predicate, pure key-expr match, always true (legacy behaviour).
fn eval_predicate(
    pred: &Option<Predicate>,
    payload: &Value,
    zones: &HashMap<String, HashSet<String>>,
) -> bool {
    match pred {
        None => true,
        Some(p) => eval_tree(p, payload, zones),
    }
}

/// Recursively walk the typed `Predicate` tree, failing closed on any
/// unsupported node. Unsupported operators or absent payload fields yield
/// `false` rather than fail-open.
fn eval_tree(pred: &Predicate, payload: &Value, zones: &HashMap<String, HashSet<String>>) -> bool {
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
/// `json_cmp` returning `None` on type mismatch is treated as `false` (fail-closed),
/// not as an ordering — this is explicit and never panics.
fn eval_comparison(op: Op, l: &Value, r: &Value, zones: &HashMap<String, HashSet<String>>) -> bool {
    match op {
        Op::Eq => values_equal(l, r),
        Op::Ne => !values_equal(l, r),
        // `json_cmp` type mismatch => None => false (fail-closed), explicit.
        Op::Lt => json_cmp(l, r).is_some_and(|o| o.is_lt()),
        Op::Gt => json_cmp(l, r).is_some_and(|o| o.is_gt()),
        Op::Le => json_cmp(l, r).is_some_and(|o| o.is_le()),
        Op::Ge => json_cmp(l, r).is_some_and(|o| o.is_ge()),
        Op::SameZoneAs => {
            let (Some(a), Some(b)) = (l.as_str(), r.as_str()) else {
                return false;
            };
            share_zone(zones, a, b)
        }
    }
}

fn share_zone(zones: &HashMap<String, HashSet<String>>, robot_a: &str, robot_b: &str) -> bool {
    let Some(a_zones) = zones.get(robot_a) else {
        return false;
    };
    let Some(b_zones) = zones.get(robot_b) else {
        return false;
    };
    a_zones.iter().any(|z| b_zones.contains(z))
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
        Operand::Field(name) => payload.get(name).cloned(),
        Operand::Prim(p) => {
            let field = prim_field(p);
            // `Proximity` is entity-aware: only resolve the separation distance
            // when the payload's `peer_id` matches the configured entity, so
            // `near = { entity = "8" }` does not match a different peer.
            if let PrimitiveRef::Proximity(entity) = p
                && payload.get("peer_id").and_then(|v| v.as_str()) != Some(entity.as_str())
            {
                return None;
            }
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

/// Compare two JSON values for ordering.
///
/// Returns `None` on type mismatch (e.g. number vs string) or on
/// non-finite floats. Callers treat `None` as `false` (fail-closed),
/// never as an ordering. This makes type-mismatch explicit rather than
/// panicking or assuming an ordering.
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
fn trigger_matches(
    trigger: &Trigger,
    topic: &str,
    payload: &Value,
    zones: &HashMap<String, HashSet<String>>,
) -> bool {
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
    zones: &HashMap<String, HashSet<String>>,
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
    zones: &HashMap<String, HashSet<String>>,
) -> bool {
    // An empty guard (no `all` and no `any`) must not be vacuous-true: a typo'd
    // or stripped-down `when` would otherwise fire the rule's actions every tick.
    // Fail closed instead — an empty guard can never fire.
    if when.all.is_empty() && when.any.is_empty() {
        return false;
    }
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
///
/// `ready_gate` is consumed as the readiness token: the engine flips it once its
/// initial sensor subscriptions are live, so the caller can gate `/readyz` on
/// actual subscription, not spawn.
pub async fn run_engine(
    transport: Arc<Transport>,
    store: ActiveRules,
    ready_gate: ReadyGate,
) -> zenoh::Result<()> {
    let eval_counter = ready_gate.eval_counter();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let gate_clone = ready_gate.clone();
    tokio::spawn(async move {
        let _ = rx.await;
        gate_clone.set_ready();
    });
    EvalState::new(transport, store)
        .run(eval_counter, Some(tx))
        .await
}

/// Subscribe to `zone/*/entered` and `zone/*/cleared` as a derived stream.
///
/// Unlike the old `ZoneTracker(Arc<Mutex>)` special case, these topics are now
/// treated like any sensor topic: the callbacks push `(topic, payload)` into the
/// shared `sample_tx` channel, and `EvalState::ingest` updates the zone map.
/// The returned handles are held for the engine's lifetime (drop-to-unsubscribe).
async fn zone_subscriptions(
    transport: &Transport,
    tx: tokio::sync::mpsc::Sender<(String, Value)>,
) -> zenoh::Result<Vec<tokio::task::JoinHandle<()>>> {
    let entered_pattern =
        crate::topic::Pattern::try_new(crate::topic::ZONE_ENTERED_PATTERN).unwrap();
    let entered_sub = transport.subscribe(entered_pattern).await?;
    let entered_tx = tx.clone();
    let entered_handle = tokio::spawn(async move {
        while let Ok(sample) = entered_sub.recv_async().await {
            let key = sample.key_expr().to_string();
            let payload: Value =
                serde_json::from_slice(&sample.payload().to_bytes()).unwrap_or(Value::Null);
            let _ = entered_tx.try_send((key, payload));
        }
    });

    let cleared_pattern =
        crate::topic::Pattern::try_new(crate::topic::ZONE_CLEARED_PATTERN).unwrap();
    let cleared_sub = transport.subscribe(cleared_pattern).await?;
    let cleared_tx = tx;
    let cleared_handle = tokio::spawn(async move {
        while let Ok(sample) = cleared_sub.recv_async().await {
            let key = sample.key_expr().to_string();
            let payload: Value =
                serde_json::from_slice(&sample.payload().to_bytes()).unwrap_or(Value::Null);
            let _ = cleared_tx.try_send((key, payload));
        }
    });

    Ok(vec![entered_handle, cleared_handle])
}

/// Subscribe to all distinct topics from the ruleset using managed subscribers.
/// `EvalMode` is part of the trigger metadata but does not filter the topic set:
/// Level vs Edge is decided at `tick` via `prev` (subscription is the same).
async fn subscribe_to_topics(
    transport: &Transport,
    rules: &Rules,
    tx: &tokio::sync::mpsc::Sender<(String, Value)>,
    subscribers: &mut Vec<tokio::task::JoinHandle<()>>,
    topics: &mut Vec<String>,
) -> zenoh::Result<()> {
    let mut new_topics: Vec<String> = Vec::new();
    for rule in &rules.rules {
        collect_topics(&rule.when, &mut new_topics);
    }
    new_topics.sort();
    new_topics.dedup();
    for topic in &new_topics {
        let pattern = crate::topic::Pattern::try_new(topic).map_err(|e| {
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)) as zenoh::Error
        })?;
        let sub = transport.subscribe(pattern).await?;
        let tx2 = tx.clone();
        let key = topic.clone();
        let handle = tokio::spawn(async move {
            while let Ok(sample) = sub.recv_async().await {
                let payload: Value =
                    serde_json::from_slice(&sample.payload().to_bytes()).unwrap_or(Value::Null);
                let _ = tx2.try_send((key.clone(), payload));
            }
        });
        subscribers.push(handle);
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
    let topic = match crate::topic::Topic::try_new(&action.topic) {
        Ok(t) => t,
        Err(e) => {
            warn!(action = %action.topic, error = %e, "action topic invalid — not published");
            return;
        }
    };
    let envelope = crate::transport::Envelope::Action {
        qos: action.qos,
        payload: action.payload.clone(),
    };
    if let Err(e) = transport.publish(topic, envelope).await {
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

    fn empty_zones() -> HashMap<String, HashSet<String>> {
        HashMap::new()
    }

    fn empty_state() -> EvalState {
        EvalState::empty()
    }

    #[test]
    fn none_predicate_always_true() {
        let zones = empty_zones();
        assert!(eval_predicate(&None, &json!({}), &zones));
        assert!(eval_predicate(&None, &json!({"anything": 1}), &zones));
    }

    #[test]
    fn comparison_eq_zone_resolves_payload() {
        let zones = empty_zones();
        let p = zone_eq("zone_1");
        assert!(eval_tree(&p, &json!({"zone_id": "zone_1"}), &zones));
        assert!(!eval_tree(&p, &json!({"zone_id": "zone_2"}), &zones));
    }

    #[test]
    fn comparison_lt_separation_distance() {
        let zones = empty_zones();
        let p = sep_lt(1.2);
        assert!(eval_tree(&p, &json!({"separation_distance": 1.0}), &zones));
        assert!(!eval_tree(&p, &json!({"separation_distance": 1.2}), &zones));
        assert!(!eval_tree(&p, &json!({"separation_distance": 2.0}), &zones));
    }

    #[test]
    fn proximity_uses_separation_distance_field() {
        let zones = empty_zones();
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::Proximity("human".to_string())),
            rhs: Operand::Float(1.2),
        };
        assert!(eval_tree(
            &p,
            &json!({"peer_id": "human", "separation_distance": 0.5}),
            &zones
        ));
        assert!(!eval_tree(
            &p,
            &json!({"peer_id": "human", "separation_distance": 1.5}),
            &zones
        ));
        assert!(!eval_tree(
            &p,
            &json!({"peer_id": "other", "separation_distance": 0.5}),
            &zones
        ));
        assert!(!eval_tree(&p, &json!({"separation_distance": 0.5}), &zones));
    }

    #[test]
    fn and_all_true_or_any_true_not_negates() {
        let zones = empty_zones();
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
        let zones = empty_zones();
        // `zone_id` absent => Prim(Zone) resolves to None => false.
        assert!(!eval_tree(&zone_eq("zone_1"), &json!({"other": 1}), &zones));
        assert!(!eval_tree(&sep_lt(1.2), &json!({}), &zones));
        // And with one absent field => whole And false.
        let and = Predicate::And(vec![zone_eq("zone_1"), sep_lt(1.2)]);
        assert!(!eval_tree(&and, &json!({"zone_id": "zone_1"}), &zones));
    }

    #[test]
    fn float_equality_uses_epsilon() {
        let zones = empty_zones();
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
        let mut state = empty_state();

        // No zone data yet — fails closed.
        let p = Predicate::Comparison {
            op: Op::SameZoneAs,
            lhs: Operand::Str("robot7".to_string()),
            rhs: Operand::Str("robot8".to_string()),
        };
        assert!(!eval_tree(&p, &json!({}), state.zones_map()));

        // robot7 enters zone_a, robot8 enters zone_a — same zone.
        state.enter_zone("robot7", "zone_a");
        state.enter_zone("robot8", "zone_a");
        assert!(eval_tree(&p, &json!({}), state.zones_map()));

        // robot8 clears zone_a — no longer same.
        state.clear_zone("robot8", "zone_a");
        assert!(!eval_tree(&p, &json!({}), state.zones_map()));

        // robot8 enters zone_b, robot7 still in zone_a — different.
        state.enter_zone("robot8", "zone_b");
        assert!(!eval_tree(&p, &json!({}), state.zones_map()));

        // Both share zone_c — overlap detected even with different primary zones.
        state.enter_zone("robot7", "zone_c");
        state.enter_zone("robot8", "zone_c");
        assert!(eval_tree(&p, &json!({}), state.zones_map()));
    }

    #[test]
    fn same_zone_non_string_fails_closed() {
        let zones = empty_zones();
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
        let mut state = empty_state();

        // Empty tracker — no sharing.
        assert!(!state.share_zone("a", "b"));

        // One robot in a zone — no sharing yet.
        state.enter_zone("a", "z1");
        assert!(!state.share_zone("a", "b"));
        assert!(!state.share_zone("b", "a"));

        // Second robot enters the same zone — share detected.
        state.enter_zone("b", "z1");
        assert!(state.share_zone("a", "b"));
        assert!(state.share_zone("b", "a"));

        // Clear zone — no longer shared.
        state.clear_zone("b", "z1");
        assert!(!state.share_zone("a", "b"));

        // Clear removes empty entry.
        state.clear_zone("a", "z1");
        assert!(!state.share_zone("a", "b"));

        // Unknown robot fails closed.
        assert!(!state.share_zone("a", "c"));
    }

    #[test]
    fn level_trigger_fires_each_tick_while_true() {
        let zones = empty_zones();
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
            json!({"peer_id": "7", "separation_distance": 0.5}),
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
        let zones = empty_zones();
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

    #[test]
    fn field_operand_reads_named_payload_field() {
        let zones = empty_zones();
        let mut prev = HashMap::new();
        // Eq: payload["pressed"] == true (fails closed when absent)
        let eq = Trigger {
            topic: "robot/7/bumper".into(),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Field("pressed".into()),
                rhs: Operand::Bool(true),
            }),
            mode: EvalMode::Edge,
        };
        let w = When {
            all: vec![eq],
            any: vec![],
        };
        let mut latest = HashMap::new();
        latest.insert("robot/7/bumper".into(), json!({"pressed": false}));
        // baseline: false (no fire on first observation)
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
        // transition false→true → fire
        latest.insert("robot/7/bumper".into(), json!({"pressed": true}));
        assert!(when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
        // holding true → no re-fire (edge)
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
        // absent field fails closed (a raw-rule typo must not read `null` as false)
        let mut prev2 = HashMap::new();
        let mut latest2 = HashMap::new();
        latest2.insert("robot/7/bumper".into(), json!({"prssed": true}));
        // baseline absent → Eq(absent) fails closed, stays false
        assert!(!when_satisfied_with_prev(
            &w, &latest2, &mut prev2, 0, &zones
        ));
        assert!(!when_satisfied_with_prev(
            &w, &latest2, &mut prev2, 0, &zones
        ));
    }

    #[test]
    fn empty_when_never_fires() {
        let zones = empty_zones();
        let mut prev = HashMap::new();
        let w = When {
            all: vec![],
            any: vec![],
        };
        let latest = HashMap::new();
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
    }

    #[test]
    fn proximity_ignores_other_peer() {
        let zones = empty_zones();
        let mut prev = HashMap::new();
        let p = Trigger {
            topic: "robot/7/proximity".into(),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity("7".into())),
                rhs: Operand::Float(1.2),
            }),
            mode: EvalMode::Level,
        };
        let w = When {
            all: vec![p],
            any: vec![],
        };
        let mut latest = HashMap::new();
        latest.insert(
            "robot/7/proximity".into(),
            json!({"peer_id": "8", "separation_distance": 0.1}),
        );
        assert!(!when_satisfied_with_prev(&w, &latest, &mut prev, 0, &zones));
    }

    #[tokio::test]
    async fn demo_rule_fires_on_bumper_pressed() {
        let zones = empty_zones();
        let mut prev = HashMap::new();
        let rules = crate::config::ActiveRules::bootstrap_demo("7")
            .current()
            .await;
        let e_stop = rules
            .rules
            .iter()
            .find(|r| r.name == "e-stop-on-bumper")
            .expect("demo e-stop rule present");
        // Both `all` triggers are Level mode: they re-evaluate every tick, so
        // the demo rule fires whenever the payloads are present and true.
        let mut latest = HashMap::new();
        latest.insert("robot/7/local/bumper".into(), json!({"pressed": true}));
        latest.insert("robot/7/local/imu".into(), json!({"speed_mps": 0.5}));
        let fires = when_satisfied_with_prev(&e_stop.when, &latest, &mut prev, 0, &zones);
        assert!(fires);
        // Level triggers keep firing every tick while true.
        assert!(when_satisfied_with_prev(
            &e_stop.when,
            &latest,
            &mut prev,
            0,
            &zones
        ));
        // A typo'd/absent field must fail closed, not read as false.
        let mut latest2 = HashMap::new();
        latest2.insert("robot/7/local/bumper".into(), json!({"pressd": true}));
        latest2.insert("robot/7/local/imu".into(), json!({"speed_mps": 0.5}));
        assert!(!when_satisfied_with_prev(
            &e_stop.when,
            &latest2,
            &mut prev,
            0,
            &zones
        ));
    }

    #[test]
    fn eval_state_ingest_and_tick_level() {
        // Pure EvalState ingest + tick: Level trigger fires every tick while true.
        let mut state = EvalState::empty();
        let rules = crate::rules::Rules {
            rules: vec![crate::rules::Rule {
                name: "level-test".into(),
                when: When {
                    all: vec![Trigger {
                        topic: "sensor/a".into(),
                        pred: None,
                        mode: EvalMode::Level,
                    }],
                    any: vec![],
                },
                actions: vec![Action {
                    topic: "actuator/out".into(),
                    qos: crate::rules::Qos::Reliable,
                    payload: json!({"fired": true}),
                }],
            }],
        };
        state.ingest_sample("sensor/a", json!({"v": 1}));
        let a1 = state.tick(&rules);
        assert_eq!(a1.len(), 1);
        let a2 = state.tick(&rules);
        assert_eq!(a2.len(), 1);
    }

    #[test]
    fn eval_state_edge_via_tick() {
        let mut state = EvalState::empty();
        let rules = crate::rules::Rules {
            rules: vec![crate::rules::Rule {
                name: "edge-test".into(),
                when: When {
                    all: vec![Trigger {
                        topic: "sensor/b".into(),
                        pred: Some(Predicate::Comparison {
                            op: Op::Eq,
                            lhs: Operand::Field("pressed".into()),
                            rhs: Operand::Bool(true),
                        }),
                        mode: EvalMode::Edge,
                    }],
                    any: vec![],
                },
                actions: vec![Action {
                    topic: "actuator/out".into(),
                    qos: crate::rules::Qos::Reliable,
                    payload: json!({"fired": true}),
                }],
            }],
        };
        // First ingest false -> no fire
        state.ingest_sample("sensor/b", json!({"pressed": false}));
        assert_eq!(state.tick(&rules).len(), 0);
        // false->true fires
        state.ingest_sample("sensor/b", json!({"pressed": true}));
        assert_eq!(state.tick(&rules).len(), 1);
        // true->true no fire
        assert_eq!(state.tick(&rules).len(), 0);
        // true->false fires
        state.ingest_sample("sensor/b", json!({"pressed": false}));
        assert_eq!(state.tick(&rules).len(), 1);
    }

    #[test]
    fn eval_state_zone_via_ingest() {
        let mut state = EvalState::empty();
        state.ingest("zone/cell-1/entered".into(), json!({"robot_id": "r1"}));
        state.ingest("zone/cell-1/entered".into(), json!({"robot_id": "r2"}));
        assert!(state.share_zone("r1", "r2"));
        state.ingest("zone/cell-1/cleared".into(), json!({"robot_id": "r2"}));
        assert!(!state.share_zone("r1", "r2"));
    }

    #[test]
    fn json_cmp_type_mismatch_is_none_explicit() {
        // Type mismatch (number vs string) returns None, not an ordering.
        assert_eq!(json_cmp(&json!(1), &json!("a")), None);
        // Same-type ordering is Some.
        assert!(json_cmp(&json!(1), &json!(2)).unwrap().is_lt());
        assert!(json_cmp(&json!("a"), &json!("b")).unwrap().is_lt());
        // Ordering predicates treat None as false (fail-closed).
        let zones = empty_zones();
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Int(1),
            rhs: Operand::Str("a".into()),
        };
        assert!(!eval_tree(&p, &json!({}), &zones));
    }

    #[test]
    fn eval_state_hot_swap_clears_prev() {
        let mut state = EvalState::empty();
        let r1 = Arc::new(crate::rules::Rules {
            rules: vec![crate::rules::Rule {
                name: "r".into(),
                when: When {
                    all: vec![Trigger {
                        topic: "sensor/x".into(),
                        pred: Some(Predicate::Comparison {
                            op: Op::Eq,
                            lhs: Operand::Field("v".into()),
                            rhs: Operand::Int(1),
                        }),
                        mode: EvalMode::Edge,
                    }],
                    any: vec![],
                },
                actions: vec![Action {
                    topic: "act".into(),
                    qos: crate::rules::Qos::Reliable,
                    payload: json!({}),
                }],
            }],
        });
        let r2 = Arc::new(crate::rules::Rules {
            rules: vec![crate::rules::Rule {
                name: "r2".into(),
                when: When {
                    all: vec![Trigger {
                        topic: "sensor/y".into(),
                        pred: None,
                        mode: EvalMode::Level,
                    }],
                    any: vec![],
                },
                actions: vec![Action {
                    topic: "act2".into(),
                    qos: crate::rules::Qos::Reliable,
                    payload: json!({}),
                }],
            }],
        });
        state.ingest_sample("sensor/x", json!({"v": 0}));
        assert_eq!(state.tick_with_arc(r1.clone()).len(), 0);
        state.ingest_sample("sensor/x", json!({"v": 1}));
        assert_eq!(state.tick_with_arc(r1.clone()).len(), 1);
        // Swap to new ruleset: prev should clear, so next tick with new topic should not have stale baseline.
        state.ingest_sample("sensor/y", json!({"any": 1}));
        // After swap, first tick of r2 with Level should fire (Level always fires when present)
        let fired = state.tick_with_arc(r2.clone());
        assert_eq!(fired.len(), 1);
        // But edge baseline was cleared, so r1's old edge state is gone.
        // Tick r1 again should treat as first observation (no fire)
        state.ingest_sample("sensor/x", json!({"v": 1}));
        assert_eq!(state.tick_with_arc(r1.clone()).len(), 0);
    }
}
