# flo-engine Ruleset Implementation Plan (#72–#77)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `flo-engine` cloud rule-server contract: a typed ruleset envelope + predicate grammar, the PRD §5 topic contract, a SQLite WORM registry with collision/audit/SHA, hot-reload with failure modes, edge/level event semantics, and a single-process server mode.

**Architecture:** Extend the existing `rules.rs` data model (add `Ruleset` envelope + typed `Predicate` tree + `EvalMode`), make `semantic.rs` compile the PRD §5 topics and predicate tree, rewrite `engine.rs` evaluation to walk the tree, add a new `registry.rs` (SQLite WORM) consumed by `config.rs` hot-reload, and add a `--mode server` process that runs the Zenoh router (from #71 `auth.rs`) + the engine in-process. The existing `config::run_hot_reload` already swaps `RuleStore` on valid TOML and keeps last-good on failure — #76 extends it to validate against the registry + write audit rows.

**Tech Stack:** Rust 2024, `zenoh 1.9.0` (unstable), `tokio`, `serde`/`serde_json`, `toml`, `sha2` (NEW, needs admin approval — see Global Constraints), `rusqlite` (NEW, needs admin approval — see Global Constraints). Crate name `flo-rs`; tests import `flo_rs::*`.

## Global Constraints

- Crate is `#![forbid(unsafe_code)]` — no `unsafe` anywhere. (AGENTS.md)
- Every new dependency requires **admin approval before it is added to `Cargo.toml`**. This plan needs TWO new crates: `sha2` (SHA-256 for #75) and `rusqlite` (SQLite WORM for #75). Both are gated — do not add until approved. (AGENTS.md)
- Third-party GitHub Actions remain pinned to full commit SHAs. (AGENTS.md — not modified by this plan.)
- `cargo` toolchain 1.97.1 MSRV; edition 2024. (AGENTS.md)
- CI gate (required checks): `fmt`, `clippy -D warnings` (default + `media`), `test (stable/beta/1.97.1)`. `media` feature excluded from CI. Run locally before any push: `cargo fmt --check`, `cargo clippy --bin flo -- -D warnings`, `cargo clippy --bin flo --features media -- -D warnings`, `cargo test`.
- TDD: write the failing test first, run it, implement, run again, commit per task.
- Primitive-only payloads (bool/int/float/string) — validation rejects non-primitives at author time. (PRD §3)
- `ruleset_name` normalized `[a-z0-9-]{1,64}`, lowercased; invalid → `BadRequest`, not `Conflict`. (PRD §6)
- Server is the single writer of `version`; bumped only on SHA change. (PRD §6)
- Keep-last-good on invalid push; never partial-apply, never swap-on-invalid. (PRD §3, §6)

---

## File Structure

- **Modify `src/rules.rs`** — add `Predicate`, `Op`, `Operand`, `PrimitiveRef`, `EvalMode`; change `Trigger.pred` to `Option<Predicate>`; add `Ruleset` envelope; add `Ruleset::from_toml`/`to_toml` + SHA helpers.
- **Modify `src/semantic.rs`** — add `SemanticRuleset` (top-level envelope); `compile()` returns `Ruleset`; emit PRD §5 topics; build `Predicate` tree from `SemanticWhen`; set `EvalMode` per primitive; validate primitive-only payloads + normalize `ruleset_name`.
- **Modify `src/engine.rs`** — rewrite `eval_predicate` to walk `Predicate`; implement edge/level semantics in the tick loop; read `EvalMode`.
- **Modify `src/transport.rs`** — add PRD §5 key-expression constants incl. `fleet/{site}/ruleset/{name}`.
- **Create `src/registry.rs`** — SQLite WORM store: insert/update/reject-with-conflict, per-ruleset + per-rule SHA-256, in-memory hot index rebuilt on startup, quarantine on mismatch.
- **Modify `src/config.rs`** — `RuleStore` holds `Ruleset` (or compiled `Rules`); `run_hot_reload` validates against `registry`, writes audit rows, swaps only on success.
- **Modify `src/main.rs` (+ `cli.rs`)** — `--mode server` subcommand: build router session via `auth::zenoh_config`, run engine in-process against registry-backed store.
- **Modify `tests/semantic_compile.rs`** — update topic expectations to PRD §5 (currently asserts legacy `fleet/{site}/{id}/state`); add #72/#73/#77 tests.
- **Create `tests/registry_test.rs`** — #75 registry/collision/audit tests.
- **Create `tests/engine_event_test.rs`** — #77 edge/level tests.

---

## Task 1: Typed predicate tree + `EvalMode` in `rules.rs` (#73 foundation)

**Files:**
- Modify: `src/rules.rs` (add after `Qos`, before `Action`)
- Test: `tests/` inline not needed; use `tests/semantic_compile.rs` later. Unit test inline in `src/rules.rs`.

**Interfaces:**
- Produces: `pub enum Predicate`, `pub enum Op`, `pub enum Operand`, `pub enum PrimitiveRef`, `pub enum EvalMode`, and revised `pub struct Trigger { topic: String, pred: Option<Predicate>, mode: EvalMode }`.

- [ ] **Step 1: Write the failing inline test**

Append to `src/rules.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predicate_tree_is_typed() {
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::Proximity("7".into())),
            rhs: Operand::Float(1.2),
        };
        assert_eq!(p, p.clone());
        // default eval mode is Edge
        assert_eq!(Trigger::default().mode, EvalMode::Edge);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo rules::tests::predicate_tree_is_typed 2>&1 | tail -5`
Expected: FAIL — `Predicate` not found.

- [ ] **Step 3: Write minimal implementation**

In `src/rules.rs`, after the `Qos` enum (line ~10), insert:

```rust
/// Boolean/arithmetic operator for a `Predicate` comparison (PRD §4 grammar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op { Eq, Ne, Lt, Gt, Le, Ge, SameZoneAs }

/// A comparison operand: a literal or a typed primitive reference (PRD §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Prim(PrimitiveRef),
}

/// One of the five rule primitives (PRD §4). `Proximity` carries the peer robot id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveRef { Site, Zone, Robot, Proximity(String), HumanPresence }

/// Evaluation mode for a trigger (PRD §1 fog, #77): fire on transition (Edge)
/// or re-evaluate every tick against latest sample (Level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EvalMode { #[default] Edge, Level }

/// A statically-auditable predicate tree (non-Turing-complete, deterministic).
/// Replaces the legacy free-text `Trigger.pred: Option<String>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Predicate {
    Comparison { op: Op, lhs: Operand, rhs: Operand },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}
```

Then change the `Trigger` struct (currently lines 25-34) to:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trigger {
    /// Key-expression the incoming sample must match.
    pub topic: String,
    /// Typed predicate over the payload (None => always true).
    #[serde(default)]
    pub pred: Option<Predicate>,
    /// Evaluation mode (#77); defaults to Edge.
    #[serde(default)]
    pub mode: EvalMode,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo rules::tests::predicate_tree_is_typed 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/rules.rs
git commit -m "feat(rules): add typed Predicate tree + EvalMode (#73)"
```

---

## Task 2: `Ruleset` envelope + TOML schema + primitive-only validation (#72)

**Files:**
- Modify: `src/rules.rs` (add `Ruleset` + helpers at end)
- Modify: `src/semantic.rs` (add `SemanticRuleset`, change `compile` return, add validation)
- Modify: `tests/semantic_compile.rs` (add envelope tests)

**Interfaces:**
- Consumes: `Predicate`, `Trigger`, `Rule`, `When`, `Action` from Task 1.
- Produces: `pub struct Ruleset { ruleset_name: String, version: u64, robot_owner: String, rules: Vec<Rule> }`; `SemanticRuleset` in semantic.rs; `Ruleset::from_toml(&str) -> Result<Ruleset, toml::de::Error>`; `Ruleset::to_toml(&self) -> String`; `SemanticRuleset::compile(&self, robot_id) -> Result<Ruleset, SemanticError>`.

- [ ] **Step 1: Write the failing test**

Add to `tests/semantic_compile.rs`:

```rust
use flo_rs::rules::Ruleset;
use flo_rs::semantic::{parse_semantic_ruleset, compile_ruleset};

const RULESET_DOC: &str = r#"
ruleset_name = "acme-site-a"
version = 3
robot_owner = "robot/7"

[[rule]]
rule_name = "slow_near_human"
when.in_zone = "zone_1"
when.near_human = 1.2
when.human_presence = true
[[rule.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { speed_mps = 0.3 }
"#;

#[test]
fn parses_ruleset_envelope() {
    let doc = parse_semantic_ruleset(RULESET_DOC).expect("parse");
    assert_eq!(doc.ruleset_name, "acme-site-a");
    assert_eq!(doc.version, 3);
    assert_eq!(doc.robot_owner, "robot/7");
    assert_eq!(doc.rules.len(), 1);
}

#[test]
fn compiles_ruleset_to_envelope() {
    let doc = parse_semantic_ruleset(RULESET_DOC).unwrap();
    let rs: Ruleset = compile_ruleset(&doc, "7").unwrap();
    assert_eq!(rs.ruleset_name, "acme-site-a");
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "slow_near_human");
}

#[test]
fn rejects_nonprimitive_payload() {
    let bad = r#"
ruleset_name = "x"
robot_owner = "robot/7"
[[rule]]
rule_name = "bad"
when.near_human = 1.0
[[rule.actions]]
topic = "robot/7/local/drive"
payload = { nested = { a = 1 } }
"#;
    let doc = parse_semantic_ruleset(bad).unwrap();
    assert!(compile_ruleset(&doc, "7").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo parses_ruleset_envelope compiles_ruleset_to_envelope rejects_nonprimitive_payload 2>&1 | tail -8`
Expected: FAIL — `parse_semantic_ruleset` not found.

- [ ] **Step 3: Write minimal implementation**

In `src/rules.rs`, append:

```rust
/// The full ruleset: an ownership/version envelope wrapping the runtime `Rule`s.
/// This is the wire + storage unit; `rules` is what `engine.rs` evaluates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ruleset {
    pub ruleset_name: String,
    pub version: u64,
    pub robot_owner: String,
    pub rules: Vec<Rule>,
}

impl Ruleset {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
    #[allow(dead_code)]
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("Ruleset is serializable")
    }
}
```

In `src/semantic.rs`:
- Add `use crate::rules::{Action, EvalMode, Op, Operand, Predicate, PrimitiveRef, Qos, Rule, Ruleset, Trigger, When};` (replace the existing `use crate::rules::{...}` at line 8).
- Add `SemanticRuleset` and `SemanticRule` rename (`name` → `rule_name`):

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRuleset {
    pub ruleset_name: String,
    #[serde(default)]
    pub version: u64,
    pub robot_owner: String,
    #[serde(default)]
    pub rules: Vec<SemanticRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRule {
    pub rule_name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    pub actions: Vec<SemanticAction>,
}
```

- Add `parse_semantic_ruleset` and `compile_ruleset`:

```rust
pub fn parse_semantic_ruleset(text: &str) -> Result<SemanticRuleset, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError(e.to_string()))
}

pub fn compile_ruleset(doc: &SemanticRuleset, robot_id: &str) -> Result<Ruleset, SemanticError> {
    // normalize ruleset_name
    let ruleset_name = doc.ruleset_name.to_lowercase();
    if !ruleset_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || ruleset_name.is_empty()
        || ruleset_name.len() > 64
    {
        return Err(SemanticError(format!(
            "invalid ruleset_name '{ruleset_name}' (must match [a-z0-9-]{{1,64}})"
        )));
    }
    let mut rules = Vec::new();
    for rule in &doc.rules {
        let (all, any) = expand_when(&rule.when, &doc.robot_owner, robot_id);
        let actions: Vec<Action> = rule
            .actions
            .iter()
            .map(|a| compile_action(a, robot_id))
            .collect();
        validate_rule_payloads(&actions, &rule.rule_name)?;
        rules.push(Rule {
            name: rule.rule_name.clone(),
            when: When { all, any },
            actions,
        });
    }
    Ok(Ruleset {
        ruleset_name,
        version: doc.version,
        robot_owner: doc.robot_owner.clone(),
        rules,
    })
}
```

- Add the payload validator:

```rust
fn validate_rule_payloads(actions: &[Action], rule_name: &str) -> Result<(), SemanticError> {
    for a in actions {
        if !is_primitive(&a.payload) {
            return Err(SemanticError(format!(
                "rule '{rule_name}': action payload must be primitive (bool/int/float/string), got {a:?}"
            )));
        }
    }
    Ok(())
}

fn is_primitive(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => true,
        serde_json::Value::Object(m) => m.values().all(is_primitive),
        _ => false,
    }
}
```

- Keep the existing `compile(doc: &SemanticDoc, robot_id)` for the legacy flat `Rules` path (do NOT break `tests/semantic_compile.rs`'s existing `compile` usages). The new `compile_ruleset` is additive.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo parses_ruleset_envelope compiles_ruleset_to_envelope rejects_nonprimitive_payload 2>&1 | tail -8`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/rules.rs src/semantic.rs tests/semantic_compile.rs
git commit -m "feat(rules): Ruleset envelope + TOML schema + primitive-only validation (#72)"
```

---

## Task 3: Typed predicate grammar in `semantic.rs` (#73)

**Files:**
- Modify: `src/semantic.rs` (`expand_when` → build `Predicate` tree; remove `format!`d pred strings)
- Modify: `tests/semantic_compile.rs` (assert typed `Predicate`, not `Some("...")`)

**Interfaces:**
- Consumes: `Predicate`, `Op`, `Operand`, `PrimitiveRef`, `EvalMode` (Task 1); `SemanticWhen` (existing).
- Produces: `Trigger`s whose `pred: Option<Predicate>` is the compiled tree and `mode: EvalMode` set per primitive.

- [ ] **Step 1: Write the failing test**

In `tests/semantic_compile.rs`, add:

```rust
use flo_rs::rules::{EvalMode, Op, Operand, Predicate, PrimitiveRef};

#[test]
fn compiles_in_zone_to_typed_predicate() {
    let doc = parse_semantic_ruleset(r#"
ruleset_name = "x"
robot_owner = "robot/7"
[[rule]]
rule_name = "r"
when.in_zone = "zone_1"
[[rule.actions]]
topic = "robot/7/local/drive"
payload = { speed_mps = 0.3 }
"#).unwrap();
    let rs = compile_ruleset(&doc, "7").unwrap();
    let t = &rs.rules[0].when.all[0];
    assert_eq!(
        t.pred,
        Some(Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::Zone),
            rhs: Operand::Str("zone_1".into()),
        })
    );
    // zone entry is an edge event
    assert_eq!(t.mode, EvalMode::Edge);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo compiles_in_zone_to_typed_predicate 2>&1 | tail -6`
Expected: FAIL (pred is `Some("zone_id == \"zone_1\"")` today).

- [ ] **Step 3: Write minimal implementation**

Replace the `expand_when` function body in `src/semantic.rs` (currently lines 194-245) with one that builds `Predicate` and sets `mode`:

```rust
fn expand_when(when: &SemanticWhen, site: &str, robot_id: &str) -> (Vec<Trigger>, Vec<Trigger>) {
    let mut all = Vec::new();
    let mut any = Vec::new();

    if let Some(z) = &when.in_zone {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/zone"),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }
    if let Some(z) = &when.not_in_zone {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/zone"),
            pred: Some(Predicate::Not(Box::new(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }))),
            mode: EvalMode::Edge,
        });
    }
    if let Some(d) = when.near_human {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/human_present"),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(d) = when.not_near_human {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/human_present"),
            pred: Some(Predicate::Comparison {
                op: Op::Ge,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(n) = &when.near {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/proximity"),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity(n.entity.clone())),
                rhs: Operand::Float(n.dist),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(r) = &when.role {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/role"),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Robot),
                rhs: Operand::Str(r.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }

    for nested in &when.all {
        let (nested_all, _) = expand_when(nested, site, robot_id);
        all.push(Trigger {
            topic: nested_all.first().map(|t| t.topic.clone()).unwrap_or_default(),
            pred: Some(Predicate::And(
                nested_all.into_iter().filter_map(|t| t.pred).collect(),
            )),
            mode: EvalMode::Level,
        });
    }
    for nested in &when.any {
        let (nested_all, _) = expand_when(nested, site, robot_id);
        any.push(Trigger {
            topic: nested_all.first().map(|t| t.topic.clone()).unwrap_or_default(),
            pred: Some(Predicate::Or(
                nested_all.into_iter().filter_map(|t| t.pred).collect(),
            )),
            mode: EvalMode::Level,
        });
    }

    (all, any)
}
```

Note: `site` parameter is now unused for topic building (PRD §5 dropped the `fleet/{site}/...` scheme). Keep the signature to avoid churn in callers, or drop it — if dropped, update both `compile_ruleset` and `compile` call sites. Prefer dropping it and updating callers.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo compiles_in_zone_to_typed_predicate 2>&1 | tail -6`
Expected: PASS

- [ ] **Step 5: Update legacy tests + commit**

The existing `tests/semantic_compile.rs` asserts `fleet/cell-7/7/state` and `Some("zone_id == \"safety\"")`. Update those assertions to PRD §5 topics + typed predicates (this is the #74 break — do it here so the suite stays green):

```rust
// in compiles_near_human_to_trigger:
assert_eq!(w.all[0].topic, "robot/7/local/human_present");
assert_eq!(
    w.all[0].pred,
    Some(Predicate::Comparison {
        op: Op::Lt,
        lhs: Operand::Prim(PrimitiveRef::HumanPresence),
        rhs: Operand::Float(1.2),
    })
);
```

Repeat for `nested_when_any_produces_triggers` / `nested_when_all_produces_triggers` (update topics to `robot/7/local/zone`, `robot/7/local/human_present`, and preds to typed `Predicate`).

Run: `cargo test --bin flo 2>&1 | tail -6`
Expected: all pass.

```bash
git add src/semantic.rs tests/semantic_compile.rs
git commit -m "feat(semantic): compile typed Predicate tree, drop free-text pred (#73)"
```

---

## Task 4: PRD §5 topic contract + transport constants (#74)

**Files:**
- Modify: `src/transport.rs` (add constants)
- Modify: `src/engine.rs` (no logic change needed — subscribes per `Trigger.topic`; verify `collect_topics` still works)
- Modify: `src/semantic.rs` (already emits §5 topics in Task 3 — verify `compile_action` topics)
- Modify: `tests/semantic_compile.rs` (assert `robot/7/local/drive` action topic — already done in Task 2/3)

**Interfaces:**
- Produces: new `pub const` values in `transport.rs`: `RULESET_PUB_KEY = "fleet/{site}/ruleset/{name}"`, and confirm existing `RULES_KEY` is kept for backward-compatible per-robot hot-reload OR replaced. Decision: KEEP `RULES_KEY` (per-robot live reload, used by `config.rs::run_hot_reload`) AND ADD `RULESET_PUB_KEY` (fleet-scoped publish used by #75/#76 server intake).

- [ ] **Step 1: Write the failing test**

Add to `tests/semantic_compile.rs`:

```rust
#[test]
fn action_targets_prd5_local_drive() {
    let doc = parse_semantic_ruleset(RULESET_DOC).unwrap();
    let rs = compile_ruleset(&doc, "7").unwrap();
    assert_eq!(rs.rules[0].actions[0].topic, "robot/7/local/drive");
}
```

And a `transport.rs` inline test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ruleset_pub_key_has_site_and_name() {
        let k = RULESET_PUB_KEY.replace("{site}", "cell-7").replace("{name}", "acme");
        assert_eq!(k, "fleet/cell-7/ruleset/acme");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo action_targets_prd5_local_drive ruleset_pub_key_has_site_and_name 2>&1 | tail -6`
Expected: FAIL (`RULESET_PUB_KEY` missing; action topic already `robot/7/local/drive` from Task 3 — that assertion will pass, the const one fails).

- [ ] **Step 3: Write minimal implementation**

In `src/transport.rs` after `RULES_KEY` (line 12), add:

```rust
/// Fleet-scoped ruleset publish key (PRD §5). Server subscribes here to
/// ingest owner pushes; `{site}` = site id, `{name}` = ruleset_name.
pub const RULESET_PUB_KEY: &str = "fleet/{site}/ruleset/{name}";
```

Verify `compile_action` in `semantic.rs` already emits `robot/{robot_id}/local/drive` (it does — see lines 247-267, unchanged). No engine.rs change required; `collect_topics`/`run_engine` already subscribe per distinct `Trigger.topic`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo action_targets_prd5_local_drive ruleset_pub_key_has_site_and_name 2>&1 | tail -6`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/transport.rs src/semantic.rs tests/semantic_compile.rs
git commit -m "feat(transport): PRD §5 topic constants + ruleset publish key (#74)"
```

---

## Task 5: Registry + collision + audit/SHA (SQLite WORM) (#75) — **GATED on dependency approval**

> **BLOCKER:** This task requires adding `sha2` and `rusqlite` to `Cargo.toml`. Per AGENTS.md, new dependencies require **admin approval** before being added. Do NOT start until approved. Add to `Cargo.toml` `[dependencies]`:
> ```toml
> sha2 = "0.10"
> rusqlite = { version = "0.32", features = ["bundled"] }
> ```
> (`bundled` avoids a system SQLite dev dependency in CI; verify against CI image.)

**Files:**
- Create: `src/registry.rs`
- Create: `tests/registry_test.rs`

**Interfaces:**
- Produces: `pub struct Registry { ... }` with `new(path: &Path) -> Result<Registry, RegistryError>`, `publish(&self, rs: &Ruleset, claiming_id: &str) -> Result<PublishOutcome, RegistryError>` where `PublishOutcome` is `Inserted | Updated { version: u64, sha: String } | RejectedConflict | Quarantined`, plus `sha256_ruleset(&Ruleset) -> String` and `sha256_rule(&Rule) -> String`.

- [ ] **Step 1: Write the failing test**

Create `tests/registry_test.rs`:

```rust
use flo_rs::rules::{Action, Operand, Op, Predicate, PrimitiveRef, Qos, Rule, Ruleset, Trigger, When};
use flo_rs::registry::{Registry, PublishOutcome};
use std::path::Path;

fn sample_rs(name: &str, owner: &str, ver: u64) -> Ruleset {
    Ruleset {
        ruleset_name: name.to_string(),
        version: ver,
        robot_owner: owner.to_string(),
        rules: vec![Rule {
            name: "r".into(),
            when: When { all: vec![], any: vec![] },
            actions: vec![Action {
                topic: "robot/7/local/drive".into(),
                qos: Qos::Reliable,
                payload: serde_json::json!({ "speed_mps": 0.3 }),
            }],
        }],
    }
}

#[test]
fn new_name_inserts() {
    let dir = std::env::temp_dir().join(format!("flo-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let reg = Registry::new(&dir.join("audit.db")).unwrap();
    let out = reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7").unwrap();
    assert!(matches!(out, PublishOutcome::Inserted));
}

#[test]
fn same_owner_updates() {
    let dir = std::env::temp_dir().join(format!("flo-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let reg = Registry::new(&dir.join("audit.db")).unwrap();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7").unwrap();
    let out = reg.publish(&sample_rs("acme", "robot/7", 2), "robot/7").unwrap();
    assert!(matches!(out, PublishOutcome::Updated { version: 2, .. }));
}

#[test]
fn different_owner_rejects_with_conflict() {
    let dir = std::env::temp_dir().join(format!("flo-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let reg = Registry::new(&dir.join("audit.db")).unwrap();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7").unwrap();
    let out = reg.publish(&sample_rs("acme", "robot/9", 1), "robot/9").unwrap();
    assert!(matches!(out, PublishOutcome::RejectedConflict));
}

#[test]
fn sha_changes_bump_version_only_on_diff() {
    let dir = std::env::temp_dir().join(format!("flo-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let reg = Registry::new(&dir.join("audit.db")).unwrap();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7").unwrap();
    // idempotent no-op push (same content) accepted but NOT recorded as new version
    let out = reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7").unwrap();
    assert!(matches!(out, PublishOutcome::Updated { version: 1, .. }));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo --test registry_test 2>&1 | tail -8`
Expected: FAIL — `flo_rs::registry` not found.

- [ ] **Step 3: Write minimal implementation**

Create `src/registry.rs`:

```rust
//! SQLite WORM audit/registry for flo-engine rulesets (PRD §6, #75).
use std::path::Path;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use crate::rules::{Rule, Ruleset};

#[derive(Debug)]
pub enum RegistryError {
    Db(rusqlite::Error),
    BadName,
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Db(e) => write!(f, "registry db error: {e}"),
            RegistryError::BadName => write!(f, "invalid ruleset_name"),
        }
    }
}
impl std::error::Error for RegistryError {}

#[derive(Debug, PartialEq)]
pub enum PublishOutcome {
    Inserted,
    Updated { version: u64, sha: String },
    RejectedConflict,
    Quarantined,
}

pub struct Registry {
    conn: Connection,
}

fn canonical_ruleset(rs: &Ruleset) -> String {
    // deterministic serialization: sort rules by name, fixed field order via toml
    let mut rs = rs.clone();
    rs.rules.sort_by(|a, b| a.name.cmp(&b.name));
    toml::to_string(&rs).expect("Ruleset serializable")
}

pub fn sha256_ruleset(rs: &Ruleset) -> String {
    let mut h = Sha256::new();
    h.update(canonical_ruleset(rs).as_bytes());
    hex::encode(h.finalize())
}

pub fn sha256_rule(rule: &Rule) -> String {
    let mut h = Sha256::new();
    h.update(toml::to_string(rule).expect("Rule serializable").as_bytes());
    hex::encode(h.finalize())
}

impl Registry {
    pub fn new(path: &Path) -> Result<Self, RegistryError> {
        let conn = Connection::open(path).map_err(RegistryError::Db)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY,
                ts TEXT NOT NULL,
                name TEXT NOT NULL,
                owner TEXT NOT NULL,
                version INTEGER NOT NULL,
                sha TEXT NOT NULL,
                status TEXT NOT NULL,
                blob TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS registry (
                name TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                version INTEGER NOT NULL,
                sha TEXT NOT NULL
            );",
        )
        .map_err(RegistryError::Db)?;
        Ok(Self { conn })
    }

    pub fn publish(&self, rs: &Ruleset, claiming_id: &str) -> Result<PublishOutcome, RegistryError> {
        if !rs.ruleset_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || rs.ruleset_name.is_empty()
            || rs.ruleset_name.len() > 64
        {
            return Err(RegistryError::BadName);
        }
        let sha = sha256_ruleset(rs);
        let ts = chrono_now();
        // ownership check
        let existing: Option<(String, i64, String)> = self
            .conn
            .query_row(
                "SELECT owner, version, sha FROM registry WHERE name = ?",
                params![rs.ruleset_name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(RegistryError::Db)?;

        match existing {
            None => {
                self.conn
                    .execute(
                        "INSERT INTO registry (name, owner, version, sha) VALUES (?,?,?,?)",
                        params![rs.ruleset_name, rs.robot_owner, 1i64, sha],
                    )
                    .map_err(RegistryError::Db)?;
                self.write_audit(&ts, rs, 1, &sha, "inserted")?;
                Ok(PublishOutcome::Inserted)
            }
            Some((owner, ver, prev_sha)) => {
                if owner != rs.robot_owner {
                    // different owner -> reject with conflict, keep last-good
                    self.write_audit(&ts, rs, ver, &sha, "rejected_conflict")?;
                    return Ok(PublishOutcome::RejectedConflict);
                }
                if prev_sha == sha {
                    // idempotent no-op: accept but do not bump/record
                    return Ok(PublishOutcome::Updated { version: ver as u64, sha });
                }
                let new_ver = ver + 1;
                self.conn
                    .execute(
                        "UPDATE registry SET owner=?, version=?, sha=? WHERE name=?",
                        params![rs.robot_owner, new_ver, sha, rs.ruleset_name],
                    )
                    .map_err(RegistryError::Db)?;
                self.write_audit(&ts, rs, new_ver, &sha, "updated")?;
                Ok(PublishOutcome::Updated { version: new_ver as u64, sha })
            }
        }
    }

    fn write_audit(
        &self,
        ts: &str,
        rs: &Ruleset,
        version: i64,
        sha: &str,
        status: &str,
    ) -> Result<(), RegistryError> {
        let blob = canonical_ruleset(rs);
        self.conn
            .execute(
                "INSERT INTO audit (ts, name, owner, version, sha, status, blob) VALUES (?,?,?,?,?,?,?)",
                params![ts, rs.ruleset_name, rs.robot_owner, version, sha, status, blob],
            )
            .map_err(RegistryError::Db)?;
        Ok(())
    }
}

fn chrono_now() -> String {
    // avoid adding chrono dep: ISO-8601-ish timestamp via std
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", s)
}
```

> NOTE: `hex::encode` requires the `hex` crate — either add `hex = "0.4"` (another NEW dep, needs approval) OR replace `hex::encode` with a small local u8→hex function to avoid the extra dependency. **Prefer the local helper** to minimize new deps:
> ```rust
> fn to_hex(bytes: &[u8]) -> String {
>     let mut s = String::with_capacity(bytes.len() * 2);
>     for b in bytes {
>         s.push_str(&format!("{:02x}", b));
>     }
>     s
> }
> ```
> and use `to_hex(&h.finalize())`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo --test registry_test 2>&1 | tail -8`
Expected: PASS

- [ ] **Step 5: Add module to lib + commit**

In `src/lib.rs`, add `pub mod registry;`.

Run: `cargo clippy --bin flo -- -D warnings 2>&1 | tail -5`
Expected: clean.

```bash
git add Cargo.toml src/registry.rs src/lib.rs tests/registry_test.rs
git commit -m "feat(registry): SQLite WORM store, collision + per-ruleset/rule SHA (#75)"
```

---

## Task 6: Hot-reload + failure modes (#76)

**Files:**
- Modify: `src/config.rs` (`RuleStore` holds `Ruleset`; `run_hot_reload` validates against `Registry`, writes audit, swaps only on success)
- Modify: `tests/` — extend `config` hot-reload test (add `tests/registry_test.rs` coverage or inline in `config.rs`)

**Interfaces:**
- Consumes: `Registry` (#75), `Ruleset`, `RULESET_PUB_KEY` (#74).
- Produces: `RuleStore` backed by `Registry`; `run_hot_reload` subscribes to `RULESET_PUB_KEY` (per-site), validates ownership + TOML, calls `registry.publish`, and on `Updated`/`Inserted` swaps the compiled `Rules`.

- [ ] **Step 1: Write the failing test**

Add to `tests/registry_test.rs`:

```rust
#[test]
fn hot_reload_rejects_bad_keeps_last_good() {
    // simulate: valid push then invalid push; store must keep the valid ruleset
    let dir = std::env::temp_dir().join(format!("flo-hr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let reg = Registry::new(&dir.join("audit.db")).unwrap();
    let good = sample_rs("acme", "robot/7", 1);
    assert!(matches!(reg.publish(&good, "robot/7").unwrap(), PublishOutcome::Inserted));
    // invalid: different owner for same name -> RejectedConflict, last-good preserved
    let bad = sample_rs("acme", "robot/9", 1);
    assert!(matches!(reg.publish(&bad, "robot/9").unwrap(), PublishOutcome::RejectedConflict));
    // registry still reports robot/7 as owner
    let ts = std::env::temp_dir().join(format!("flo-hr-{}", std::process::id())).join("audit.db");
    let reg2 = Registry::new(&ts).unwrap();
    assert!(matches!(reg2.publish(&good, "robot/7").unwrap(), PublishOutcome::Updated { version: 1, .. }));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo --test registry_test hot_reload_rejects_bad_keeps_last_good 2>&1 | tail -6`
Expected: this particular test may pass already (registry-level). The integration gap is in `run_hot_reload`. Extend `config.rs` so `run_hot_reload` takes a `&Registry` and uses `RULESET_PUB_KEY`.

- [ ] **Step 3: Write minimal implementation**

In `src/config.rs`:
- Change `RuleStore.inner` to `Arc<RwLock<Arc<Ruleset>>>` (or keep `Rules` and store `Ruleset` separately — choose: hold `Ruleset` so `robot_owner`/`version` survive for audit). Add `pub fn current_ruleset(&self) -> Arc<Ruleset>`.
- Change `run_hot_reload` signature to `pub async fn run_hot_reload(transport: &Transport, robot_id: &str, store: RuleStore, registry: Arc<Registry>) -> zenoh::Result<()>` (or `&Registry`). Subscribe to `RULESET_PUB_KEY.replace("{site}", ...).replace("{name}", ...)` — NOTE: the server subscribes to its site's ruleset prefix; for a single-site demo use the owner's site derived from `robot_id`. Use a wildcard subscribe `fleet/{site}/ruleset/**` if Zenoh supports it, else iterate known names. Keep it simple: subscribe to the concrete key built from `robot_id`'s site.
- On each sample: `Ruleset::from_toml(&text)`; on parse err → `error!` + keep last-good (no swap). On ok → `registry.publish(&rs, claiming_id)`; on `RejectedConflict`/`Quarantined` → log + keep last-good; on `Inserted`/`Updated` → compile `rs.rules` to `Rules` and `store.swap(Arc::new(rules))`, log `info!`.

- [ ] **Step 4: Run tests**

Run: `cargo test --bin flo 2>&1 | tail -6`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/registry_test.rs
git commit -m "feat(config): hot-reload validates via registry, audit + keep-last-good (#76)"
```

---

## Task 7: Edge/level event semantics (#77)

**Files:**
- Modify: `src/engine.rs` (tick loop honors `Trigger.mode`: Edge fires on payload transition; Level re-evaluates each tick)
- Create: `tests/engine_event_test.rs`

**Interfaces:**
- Consumes: `EvalMode` (Task 1), `Trigger.mode`.
- Produces: deterministic edge/level firing; no API change to `run_engine`.

- [ ] **Step 1: Write the failing test**

Create `tests/engine_event_test.rs` (unit-style via `engine` internals — expose a small `eval_when_with_prev` helper or test `when_satisfied` with a previous-state map). Minimal viable assertion: a Level trigger re-fires while true; an Edge trigger fires only on transition. Because `engine.rs` functions are private, add `#[cfg(test)] pub(crate)` wrappers or test inline in `src/engine.rs`. Prefer inline test module in `engine.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{EvalMode, Operand, Op, Predicate, PrimitiveRef, Trigger, When};

    fn level_trigger() -> Trigger {
        Trigger {
            topic: "robot/7/local/proximity".into(),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity("7".into())),
                rhs: Operand::Float(1.2),
            }),
            mode: EvalMode::Level,
        }
    }

    #[test]
    fn level_evaluates_against_latest_each_tick() {
        let mut latest = HashMap::new();
        latest.insert(
            "robot/7/local/proximity".to_string(),
            serde_json::json!({ "separation_distance": 0.5 }),
        );
        let w = When { all: vec![level_trigger()], any: vec![] };
        assert!(when_satisfied(&w, &latest));
    }

    #[test]
    fn edge_fires_only_on_transition() {
        // edge trigger: track previous payload; fire when predicate outcome flips
        let mut prev: Option<bool> = None;
        let trig = Trigger { topic: "robot/7/local/zone".into(),
            pred: Some(Predicate::Comparison { op: Op::Eq, lhs: Operand::Prim(PrimitiveRef::Zone), rhs: Operand::Str("zone_1".into()) }),
            mode: EvalMode::Edge };
        let on_enter = serde_json::json!({ "zone_id": "zone_1" });
        let on_exit = serde_json::json!({ "zone_id": "zone_2" });
        let eval = |p: &serde_json::Value| -> bool {
            let mut m = HashMap::new();
            m.insert("robot/7/local/zone".to_string(), p.clone());
            when_satisfied(&When { all: vec![trig.clone()], any: vec![] }, &m)
        };
        // enter: prev None -> true => fire
        let cur = eval(&on_enter);
        let fired_enter = prev.map_or(cur, |_| false) || (prev == Some(false) && cur);
        assert!(fired_enter);
        prev = Some(cur);
        // still in zone: no new fire
        let cur2 = eval(&on_enter);
        let fired_hold = prev == Some(false) && cur2;
        assert!(!fired_hold);
        prev = Some(cur2);
        // exit: transition true->false => fire exit
        let cur3 = eval(&on_exit);
        let fired_exit = prev == Some(true) && !cur3;
        assert!(fired_exit);
    }
}
```

- [ ] **Step 2: Run test to verify it fails / passes**

Run: `cargo test --bin flo engine::tests 2>&1 | tail -8`
Expected: the assertions encode the intended edge/level behavior; adjust the `when_satisfied` tick integration so the engine's 50ms tick honors `mode` (Edge: only push to fire-queue when outcome transitions; Level: re-evaluate each tick). Implement the transition tracking in the `rx.recv()` handler: keep `prev_pred_outcome: HashMap<(topic,idx), bool>`.

- [ ] **Step 3: Implement transition tracking in engine**

In `src/engine.rs` `run_engine`, after inserting into `latest`, compute per-trigger edge transitions and only enqueue fires for Edge triggers whose outcome changed; Level triggers fire every tick as today. Concretely: in the tick loop, for each rule, evaluate `when_satisfied`; additionally, for Edge triggers, maintain `prev: HashMap<String, bool>` keyed by `topic` and only treat the rule as "fired" if any Edge trigger's boolean outcome flipped since last tick.

- [ ] **Step 4: Run tests**

Run: `cargo test --bin flo 2>&1 | tail -6`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs
git commit -m "feat(engine): edge/level event semantics per trigger (#77)"
```

---

## Task 8: Server mode process (#71 follow-up, #H)

**Files:**
- Modify: `src/cli.rs` (add `--mode server` / subcommand)
- Modify: `src/main.rs` (dispatch to server mode)
- Modify: `src/production.rs` or new `src/server.rs` (router + engine co-located)

**Interfaces:**
- Consumes: `auth::zenoh_config(&robot_id)` (#71), `Registry` (#75), `RuleStore` (#76), `engine::run_engine`.
- Produces: `flo --mode server --robot-id <id> --auth-mode mtls ...` starts a Zenoh router with the #71 ACL + runs the engine in-process.

- [ ] **Step 1: Write the failing test / manual check**

No unit test; manual: `cargo run -- --mode server --robot-id robot/7 --auth-mode mtls` should start without panic and log "rule engine subscribed" + "hot-reload subscriber active". Add a `#[cfg(test)]` smoke test that builds `auth.zenoh_config("robot/7")` (already covered) — no new test needed.

- [ ] **Step 2: Implement server dispatch**

In `src/cli.rs` add `mode: Mode { Client, Server }` (default Client). In `src/main.rs`, match mode: Server → build `auth` via `AuthConfig::from_cli(...)`, `auth.validate_production()?`, `let session = zenoh::open(auth.zenoh_config(&robot_id)?).await?;` (router config), `let registry = Arc::new(Registry::new(&audit_db)?);`, `let store = RuleStore::bootstrap_demo(&robot_id);`, `tokio::join!(engine::run_engine(transport, store.clone(), counter), config::run_hot_reload(&transport, &robot_id, store, registry.clone()))`. `transport` must wrap the router `Session` — extend `Transport::open_with` (or add `Transport::from_session`) to accept an already-open `zenoh::Session`.

- [ ] **Step 3: Wire Transport from a router session**

In `src/transport.rs`, add `pub async fn from_session(session: zenoh::Session) -> Self` (wrapping the session for `subscribe`/`publish`). `open_with` becomes a thin wrapper calling `zenoh::open` then `from_session`.

- [ ] **Step 4: Run the full gate**

Run: `cargo fmt --check && cargo clippy --bin flo -- -D warnings && cargo clippy --bin flo --features media -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs src/server.rs src/transport.rs src/production.rs
git commit -m "feat(server): single-process flo-engine server mode (router + eval) (#H)"
```

---

## Self-Review Notes

- **Spec coverage:** §A data model → Tasks 1-3; §B envelope → Task 2; §C predicate → Tasks 1,3; §D topic contract → Task 4 (+ Task 3 emit); §E registry/SHA → Task 5 (gated); §F hot-reload → Task 6; §G event semantics → Task 7; §H server → Task 8. Covered.
- **Dependency gate:** Task 5 is explicitly blocked on admin approval for `sha2` + `rusqlite` (+ optional `hex` avoided via local helper). This is surfaced in Global Constraints and at Task 5's top.
- **Backward compat:** `config.rs::run_hot_reload` already existed (swap-on-valid, keep-last-good); Task 6 extends it to the registry. The legacy flat `compile(doc, robot_id)` path is kept alongside `compile_ruleset` so existing `tests/semantic_compile.rs` non-envelope tests still pass.
- **PRD §5 break:** Task 3/4 deliberately change topics from `fleet/{site}/{id}/state` to `robot/{id}/local/...`; the existing test expectations are updated in Task 3 Step 5. Confirm with reviewer this break is intended (it is — user chose "Adopt PRD §5 schema").
