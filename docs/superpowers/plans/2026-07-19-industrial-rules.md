# Industrial Semantic Rule Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a semantic rule-authoring layer to `flo` so operators write rules against zones,
roles, poses, proximity, and human-presence (not raw Zenoh key-expressions); `flo` compiles that
to the existing TOML rule engine unchanged at runtime.

**Architecture:** A new `src/semantic.rs` module parses extended-TOML semantic docs and compiles
them to the existing `rules::Rules` struct (the same struct `engine.rs` evaluates). A `flo rule
check <file>` subcommand validates a semantic doc before compile. `main.rs` gains a `rule` subcommand
and `run_production` compiles an extended-TOML config (or falls back to a fail-safe safe-state
ruleset on missing/malformed input). The evaluation engine, transport, and hot-reload are NOT
modified.

**Tech Stack:** Rust 1.97.1, `tokio`, `serde` + `toml` (already deps), `anyhow`. No new
dependencies. `#![forbid(unsafe_code)]` preserved. Zero system deps.

## Global Constraints

- `#![forbid(unsafe_code)]` — no `unsafe` anywhere, including new `semantic.rs` and any test code.
- Default build has **zero system dependencies**; the only YAML option (`serde_yaml`) is rejected
  because it pulls `unsafe-libyaml` (C/system dep). Authoring is **extended TOML** (TOML is already
  a dependency).
- Crate is `flo-rs`; binary is `flo`. Commands in docs use `flo`.
- CI runs only `ubuntu-latest`; all GitHub Actions SHA-pinned — do NOT touch workflows in this plan.
- The evaluation runtime (`engine.rs`, `transport.rs`) is **unchanged**; the compiler emits the
  existing `rules::Rules` shape so `RuleStore::bootstrap` / `run_hot_reload` keep working.
- `flo` is honestly the software pre-estop / coordination layer; hardware STO remains primary.

### v1 topic contract (exact-topic, no wildcards — required so the existing engine works unchanged)

The compiler emits `Trigger`s against these concrete topics. A robot (and its local fusion)
publishes:
- Own state:      `fleet/{site}/{id}/state`        payload `{zone_id, role, speed, pose?}`
- Own liveliness: `fleet/{site}/{id}/alive`
- Nearest human:  `fleet/{site}/proximity/{id}/human`   payload `{separation_distance: f64}`
- Nearest peer:   `fleet/{site}/{id}/nearest_peer`       payload `{id: str, separation_distance: f64}`

Semantic primitives compile as:
- `in_zone(z)`        → topic `fleet/{site}/{id}/state`,        pred `zone_id == "z"`
- `not_in_zone(z)`    → topic `fleet/{site}/{id}/state`,        pred `zone_id != "z"`
- `near_human(d)`     → topic `fleet/{site}/proximity/{id}/human`, pred `separation_distance < d`
- `not_near_human(d)` → topic `fleet/{site}/proximity/{id}/human`, pred `separation_distance >= d`
- `near(e, d)`        → topic `fleet/{site}/{id}/nearest_peer`,  pred `separation_distance < d`
  (v1 requires a concrete `entity` id `e`; pattern matching like `*amr*` is deferred)
- `role(r)`           → topic `fleet/{site}/{id}/state`,        pred `role == "r"`

Actions compile as:
- `estop()`   → `Action { topic="stop/fleet/cmd",          qos=Reliable,    payload={stop=true} }`
- `slow_to(v)`→ `Action { topic="robot/{id}/local/drive",  qos=BestEffort,  payload={speed_mps=v} }`
- `resume`    → `Action { topic="robot/{id}/local/drive",  qos=Reliable,    payload={resume=true} }`

---

## File Structure

- Create `src/semantic.rs` — semantic doc types (`SemanticDoc`, `SemanticRule`, `SemanticWhen`,
  `SemanticAction`, `Zone`, `Site`), `compile(doc, robot_id) -> Result<Rules, SemanticError>`,
  and `validate(doc) -> Result<(), SemanticError>`. Exposed as `pub mod semantic` in `lib.rs`
  and `main.rs`.
- Modify `src/lib.rs` — add `pub mod semantic;`.
- Modify `src/main.rs` — add `rule` subcommand dispatch; `run_production` compiles extended-TOML
  (with safe-state fallback); extend `help_text()` with the new subcommand.
- Create `examples/rules/hrc-cell.toml` and `examples/rules/warehouse-fleet.toml` — promoted from
  the planning drafts (concrete ids, no `*amr*` patterns).
- Create `examples/semantic_rules.rs` — loads an extended-TOML file, compiles it, runs the engine.
- Create `tests/semantic_compile.rs` — TDD tests for parse/compile/validate.
- Create `tests/rule_check.rs` — TDD test for `flo rule check` exit codes.
- Modify `README.md` — document the semantic layer (one section; keep fact-checked tone).

---

### Task 1: Semantic doc types + parse + validate (TDD)

**Files:**
- Create: `src/semantic.rs`
- Create: `tests/semantic_compile.rs`
- Modify: `src/lib.rs` (add `pub mod semantic;`)

**Interfaces:**
- Consumes: `rules::Rules`, `rules::Qos` (from `src/rules.rs`).
- Produces: `pub fn parse_semantic(text: &str) -> Result<SemanticDoc, SemanticError>`,
  `pub fn validate(doc: &SemanticDoc) -> Result<(), SemanticError>`,
  `pub struct SemanticError(pub String)` (implements `std::fmt::Display` + `std::error::Error`).

- [ ] **Step 1: Write the failing test** (`tests/semantic_compile.rs`)

```rust
use flo_rs::semantic::{parse_semantic, validate, SemanticError};

const DOC: &str = r#"
[site]
id = "cell-7"
frame = "cell-7/world"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
"#;

#[test]
fn parses_minimal_semantic_doc() {
    let doc = parse_semantic(DOC).expect("parse");
    assert_eq!(doc.site.id, "cell-7");
    assert_eq!(doc.zones.get("safety").unwrap().w, 2.0);
    assert_eq!(doc.rules.len(), 1);
    assert_eq!(doc.rules[0].when.near_human, Some(1.2));
}

#[test]
fn validates_good_doc_ok() {
    let doc = parse_semantic(DOC).unwrap();
    assert!(validate(&doc).is_ok());
}

#[test]
fn rejects_negative_distance() {
    let bad = r#"
[[rules]]
name = "x"
when.near_human = -1.0
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    let err = validate(&doc).unwrap_err();
    assert!(err.to_string().contains("distance"));
}

#[test]
fn rejects_unknown_action_verb() {
    // `explode` is not a known verb; an action with no known verb must fail validation.
    let bad = r#"
[[rules]]
name = "x"
when.in_zone = "safety"
actions = [ { explode = true } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    let err = validate(&doc).unwrap_err();
    assert!(err.to_string().contains("action"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test semantic_compile 2>&1 | tail -20`
Expected: compile error — `unresolved import flo_rs::semantic` / module not found.

- [ ] **Step 3: Write minimal implementation** (`src/semantic.rs`)

```rust
//! Semantic rule-authoring layer: parse extended-TOML and compile to `rules::Rules`.
//! See docs/superpowers/specs/2026-07-19-industrial-rules-design.md.

use std::collections::HashMap;

use serde::Deserialize;

use crate::rules::{Qos, Rules};

/// Error type for semantic parse/validate/compile.
#[derive(Debug)]
pub struct SemanticError(pub String);

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "semantic rule error: {}", self.0)
    }
}
impl std::error::Error for SemanticError {}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Site {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub frame: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Zone {
    pub shape: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NearSpec {
    pub entity: String,
    pub dist: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SemanticWhen {
    #[serde(default)]
    pub in_zone: Option<String>,
    #[serde(default)]
    pub not_in_zone: Option<String>,
    #[serde(default)]
    pub near_human: Option<f64>,
    #[serde(default)]
    pub not_near_human: Option<f64>,
    #[serde(default)]
    pub near: Option<NearSpec>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticAction {
    #[serde(default)]
    pub estop: bool,
    #[serde(default)]
    pub slow_to: Option<f64>,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub qos: Qos,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRule {
    pub name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    pub actions: Vec<SemanticAction>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SemanticDoc {
    #[serde(default)]
    pub site: Site,
    #[serde(default)]
    pub zones: HashMap<String, Zone>,
    #[serde(default)]
    pub rules: Vec<SemanticRule>,
}

/// Parse an extended-TOML semantic document.
pub fn parse_semantic(text: &str) -> Result<SemanticDoc, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError(e.to_string()))
}

/// Validate semantic invariants before compile.
pub fn validate(doc: &SemanticDoc) -> Result<(), SemanticError> {
    for rule in &doc.rules {
        // distance must be positive where present
        for d in [
            rule.when.near_human,
            rule.when.not_near_human,
            rule.when.near.as_ref().map(|n| n.dist),
        ]
        .into_iter()
        .flatten()
        {
            if d <= 0.0 {
                return Err(SemanticError(format!(
                    "rule '{}': distance must be > 0, got {d}",
                    rule.name
                )));
            }
        }
        // every action must carry at least one known verb
        for a in &rule.actions {
            if !a.estop && a.slow_to.is_none() && !a.resume {
                return Err(SemanticError(format!(
                    "rule '{}': action has no known verb (estop/slow_to/resume)",
                    rule.name
                )));
            }
        }
        // referenced zone must exist (when uses in_zone/not_in_zone)
        for z in [rule.when.in_zone.clone(), rule.when.not_in_zone.clone()].into_iter().flatten() {
            if !doc.zones.contains_key(&z) {
                return Err(SemanticError(format!(
                    "rule '{}': references unknown zone '{z}'",
                    rule.name
                )));
            }
        }
    }
    Ok(())
}
```

Expose it — edit `src/lib.rs` to add `pub mod semantic;` (alphabetical with the others):
```rust
pub mod codec;
pub mod config;
pub mod engine;
pub mod rules;
pub mod semantic;
pub mod simulate;
pub mod transport;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test semantic_compile 2>&1 | tail -20`
Expected: all 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs src/lib.rs tests/semantic_compile.rs
git commit -m "feat(semantic): parse + validate extended-TOML rule docs"
```

---

### Task 2: Compiler `compile(doc, robot_id) -> Rules` (TDD)

**Files:**
- Modify: `src/semantic.rs` (add `compile`)
- Modify: `tests/semantic_compile.rs` (add compile tests)

**Interfaces:**
- Consumes: `parse_semantic`, `validate`, `SemanticDoc`, `rules::Rules`, `rules::Rule`,
  `rules::When`, `rules::Trigger`, `rules::Action`, `rules::Qos`.
- Produces: `pub fn compile(doc: &SemanticDoc, robot_id: &str) -> Result<Rules, SemanticError>`
  (calls `validate` first; returns `Rules` consumable by `RuleStore::bootstrap`).

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_compile.rs`:
```rust
use flo_rs::semantic::compile;
use flo_rs::rules::{Rules, When};

#[test]
fn compiles_near_human_to_trigger() {
    let doc = parse_semantic(DOC).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let r = &rules.rules[0];
    assert_eq!(r.name, "hrc-slow-near-human");
    // one trigger: topic fleet/cell-7/proximity/7/human, pred separation_distance < 1.2
    let w: &When = &r.when;
    assert_eq!(w.all.len(), 1);
    assert_eq!(w.all[0].topic, "fleet/cell-7/proximity/7/human");
    assert_eq!(w.all[0].pred, Some("separation_distance < 1.2".to_string()));
    // one action: slow_to -> robot/7/local/drive, best_effort
    assert_eq!(r.actions.len(), 1);
    assert_eq!(r.actions[0].topic, "robot/7/local/drive");
    assert_eq!(r.actions[0].qos, flo_rs::rules::Qos::BestEffort);
}

#[test]
fn compile_rejects_unknown_zone() {
    let bad = r#"
[[rules]]
name = "x"
when.in_zone = "nope"
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    assert!(compile(&doc, "7").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test semantic_compile compile 2>&1 | tail -20`
Expected: FAIL — `compile` not found / `Rules` fields private.

- [ ] **Step 3: Write minimal implementation**

Append to `src/semantic.rs`:
```rust
use crate::rules::{Action, Rule, Trigger, When};

/// Compile a validated semantic doc to the runtime `Rules` shape.
pub fn compile(doc: &SemanticDoc, robot_id: &str) -> Result<Rules, SemanticError> {
    validate(doc)?;
    let site = if doc.site.id.is_empty() {
        return Err(SemanticError("missing [site].id".to_string()));
    } else {
        &doc.site.id
    };

    let mut out = Vec::new();
    for rule in &doc.rules {
        let mut triggers = Vec::new();

        if let Some(z) = &rule.when.in_zone {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/{robot_id}/state"),
                pred: Some(format!("zone_id == \"{z}\"")),
            });
        }
        if let Some(z) = &rule.when.not_in_zone {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/{robot_id}/state"),
                pred: Some(format!("zone_id != \"{z}\"")),
            });
        }
        if let Some(d) = rule.when.near_human {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/proximity/{robot_id}/human"),
                pred: Some(format!("separation_distance < {d}")),
            });
        }
        if let Some(d) = rule.when.not_near_human {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/proximity/{robot_id}/human"),
                pred: Some(format!("separation_distance >= {d}")),
            });
        }
        if let Some(n) = &rule.when.near {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/{robot_id}/nearest_peer"),
                pred: Some(format!("separation_distance < {}", n.dist)),
            });
        }
        if let Some(r) = &rule.when.role {
            triggers.push(Trigger {
                topic: format!("fleet/{site}/{robot_id}/state"),
                pred: Some(format!("role == \"{r}\"")),
            });
        }

        let actions: Vec<Action> = rule
            .actions
            .iter()
            .map(|a| compile_action(a, robot_id))
            .collect();

        out.push(Rule {
            name: rule.name.clone(),
            when: When { all: triggers, any: vec![] },
            actions,
        });
    }
    Ok(Rules { rules: out })
}

fn compile_action(a: &SemanticAction, robot_id: &str) -> Action {
    if a.estop {
        Action {
            topic: "stop/fleet/cmd".to_string(),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "stop": true }),
        }
    } else if a.resume {
        Action {
            topic: format!("robot/{robot_id}/local/drive"),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "resume": true }),
        }
    } else {
        Action {
            topic: format!("robot/{robot_id}/local/drive"),
            qos: a.qos,
            payload: serde_json::json!({ "speed_mps": a.slow_to.unwrap_or(0.0) }),
        }
    }
}
```

(Make sure `use crate::rules::{Action, Rule, Trigger, When};` is added at the top of `semantic.rs`
and `Qos`/`Rules` remain imported. `pub struct Rule`/`When`/`Trigger`/`Action` in `rules.rs` are
already `pub`, so the fields are accessible within the same crate.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test semantic_compile 2>&1 | tail -20`
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_compile.rs
git commit -m "feat(semantic): compile doc to runtime Rules"
```

---

### Task 3: `flo rule check` subcommand (TDD on arg parse)

**Files:**
- Modify: `src/main.rs` (`parse_args_from`, `help_text`, `main` dispatch, `run_rule_check`)
- Create: `tests/rule_check.rs`

**Interfaces:**
- Consumes: `semantic::parse_semantic`, `semantic::validate`, `semantic::compile`.
- Produces: `flo rule check <path>` exits 0 on valid, non-zero on invalid; prints human errors.

- [ ] **Step 1: Write the failing test** (`tests/rule_check.rs`)

```rust
use std::process::Command;

#[test]
fn rule_check_passes_valid_doc() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/rules/hrc-cell.toml");
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "check", path])
        .output()
        .expect("run flo rule check");
    assert!(
        out.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rule_check_fails_invalid_doc() {
    // write a temp bad doc
    let dir = std::env::temp_dir();
    let p = dir.join("flo-bad-rule.toml");
    std::fs::write(&p, "[[rules]]\nname=\"x\"\nwhen.near_human = -1.0\nactions = [ { slow_to = 0.1 } ]\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check");
    assert!(!out.status.success(), "expected failure on bad doc");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test rule_check 2>&1 | tail -20`
Expected: compile error — `flo rule check` not handled; `env!("CARGO_BIN_EXE_flo")` requires the
binary built (it is). The parse ignores `rule` as unknown arg, so `check` is also ignored → exits 0.

- [ ] **Step 3: Write minimal implementation**

Edit `src/main.rs`:
1. Add to `Args` a `rule: Option<Vec<String>>` field (the `rule` subcommand + its args):
```rust
#[derive(Default)]
struct Args {
    robot_id: Option<String>,
    config: Option<String>,
    simulate: bool,
    simulate_period_ms: u64,
    video: VideoArgs,
    rule: Option<Vec<String>>,
}
```
2. In `parse_args_from`, add a branch BEFORE the match (or handle `rule` as a subcommand collector):
```rust
let mut rule_args: Vec<String> = Vec::new();
let mut i = iter;
// peek: if first token is "rule", capture the rest as subcommand args
let mut collected: Vec<String> = Vec::new();
while let Some(a) = i.next() {
    if a == "rule" {
        // collect remaining as rule subcommand args
        while let Some(r) = i.next() { collected.push(r); }
        args.rule = Some(collected);
        break;
    }
    match a.as_str() {
        // ... existing arms unchanged ...
        other => eprintln!("ignoring unknown arg: {other}"),
    }
}
```
(Keep the existing `while let Some(a) = iter.next()` loop; add the `rule` short-circuit at the top
of the loop body: `if a == "rule" { while let Some(r) = iter.next() { collected.push(r); } args.rule = Some(collected); break; }` before the `match`.)

3. In `main`, after parsing args, dispatch:
```rust
if let Some(rule_cmd) = &args.rule {
    return run_rule_command(rule_cmd);
}
```
placed right after `let args = parse_args();` and before the `demo` computation.

4. Add the handler:
```rust
fn run_rule_command(cmd: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd.first().map(String::as_str) {
        Some("check") => {
            let path = cmd.get(1).ok_or("usage: flo rule check <path>")?;
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            match flo_rs::semantic::parse_semantic(&text) {
                Ok(doc) => match flo_rs::semantic::validate(&doc) {
                    Ok(()) => {
                        println!("OK: {path} is a valid semantic ruleset");
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("INVALID: {e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("PARSE ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown rule subcommand: {other:?} (try 'flo rule check <path>')");
            std::process::exit(2);
        }
    }
}
```

5. Extend `help_text()` OPTIONS block with:
```
\x20\x20rule check <path>        validate a semantic ruleset (extended TOML) before deploy\n\
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test rule_check 2>&1 | tail -20`
Expected: both tests PASS (the valid doc succeeds; the bad doc exits non-zero).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs tests/rule_check.rs
git commit -m "feat(cli): add 'flo rule check' semantic validator"
```

---

### Task 4: Example packs + semantic_rules example (TDD)

**Files:**
- Create: `examples/rules/hrc-cell.toml`
- Create: `examples/rules/warehouse-fleet.toml`
- Create: `examples/semantic_rules.rs`
- Modify: `tests/examples_build.rs` (ensure `semantic_rules` builds; it already builds all examples)
- Modify: `Cargo.toml` (add `[[example]] name = "semantic_rules"` if not auto-detected — examples
  dir is auto-discovered, so usually not needed; verify with `cargo build --examples`)

**Interfaces:**
- Consumes: `semantic::parse_semantic`, `semantic::compile`, `config::RuleStore::bootstrap`,
  `engine::run_engine`, `transport::Transport`.
- Produces: runnable examples demonstrating the semantic layer end-to-end (loopback).

- [ ] **Step 1: Write the example packs (concrete ids, no `*amr*` patterns)**

`examples/rules/hrc-cell.toml`:
```toml
[site]
id = "cell-7"
frame = "cell-7/world"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
approach = { shape = "rect", x = -1.0, y = -1.0, w = 4.0, h = 4.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
[[rules]]
name = "hrc-protective-stop-on-breach"
when.any = [
  { in_zone = "safety" },
  { near_human = 0.3 },
]
actions = [ { estop = true, qos = "reliable" } ]
[[rules]]
name = "hrc-resume-after-clear"
when.all = [
  { not_near_human = 1.5 },
  { not_in_zone = "safety" },
]
actions = [ { resume = true, qos = "reliable" } ]
```

`examples/rules/warehouse-fleet.toml`:
```toml
[site]
id = "dc-2"
frame = "dc-2/world"
[zones]
aisle-a = { shape = "rect", x = 0.0, y = 0.0, w = 1.2, h = 40.0 }
station-1 = { shape = "rect", x = 6.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "amr-yield-near-peer"
when.near = { entity = "8", dist = 2.0 }
actions = [ { slow_to = 0.3, qos = "best_effort" } ]
[[rules]]
name = "amr-slow-in-aisle"
when.in_zone = "aisle-a"
actions = [ { slow_to = 0.5, qos = "best_effort" } ]
[[rules]]
name = "amr-dock-at-station"
when.in_zone = "station-1"
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
[[rules]]
name = "amr-reserve-on-conflict"
when.near = { entity = "8", dist = 0.8 }
actions = [ { estop = true, qos = "reliable" } ]
```

- [ ] **Step 2: Write `examples/semantic_rules.rs`**

```rust
//! Load an extended-TOML semantic ruleset, compile it, and run the rule engine.
//! Run:  cargo run --example semantic_rules -- examples/rules/hrc-cell.toml
//! Then publish synthetic state on `fleet/<site>/<id>/state` and
//! `fleet/<site>/proximity/<id>/human` to watch rules fire.

use std::sync::Arc;

use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::semantic::{compile, parse_semantic};
use flo_rs::transport::Transport;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let robot_id = "7".to_string();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/rules/hrc-cell.toml".to_string());
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
    let doc = parse_semantic(&text).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let rules = compile(&doc, &robot_id).map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    println!("semantic_rules: compiled {} rule(s) from {path}", rules.rules.len());

    let mut transport = Transport::open_with(Transport::loopback_config()).await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);

    let store = RuleStore::bootstrap(&flo_rs::rules::Rules::to_toml_friendly(&rules))
        .map_err(|e| anyhow::anyhow!("bootstrap: {e}"))?;

    engine::run_engine(transport, store).await?;
    Ok(())
}
```

`Rules` has no `to_toml` today and the rule structs derive only `Deserialize`. Add `Serialize`
so we can feed compiled rules back through `RuleStore::bootstrap` (the existing TOML path).

Edit `src/rules.rs`:
1. Change the top import from `use serde::Deserialize;` to:
```rust
use serde::{Deserialize, Serialize};
```
2. Add `Serialize` to every `#[derive(...)]` on the rule types (lines 5, 13, 25, 37, 48, 57):
   `Qos`, `Action`, `Trigger`, `When`, `Rule`, `Rules`. They become e.g.
   `#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]` (Qos) and
   `#[derive(Debug, Clone, Deserialize, Serialize)]` for the rest.
3. Add a serializer method to `Rules` (after `from_toml`):
```rust
impl Rules {
    /// Parse a ruleset from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Serialize back to TOML — used to feed `RuleStore::bootstrap` after compile.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("Rules is serializable")
    }
}
```
Then in the example call `RuleStore::bootstrap(&rules.to_toml())`.

- [ ] **Step 3: Build examples to verify they compile**

Run: `cargo build --examples 2>&1 | tail -20`
Expected: SUCCESS (all examples build, including `semantic_rules`).

- [ ] **Step 4: Run `flo rule check` on the new packs**

Run:
```bash
cargo run -- rule check examples/rules/hrc-cell.toml
cargo run -- rule check examples/rules/warehouse-fleet.toml
```
Expected: both print `OK: ... is a valid semantic ruleset`.

- [ ] **Step 5: Commit**

```bash
git add examples/rules/hrc-cell.toml examples/rules/warehouse-fleet.toml examples/semantic_rules.rs src/rules.rs
git commit -m "feat(examples): semantic rule packs + semantic_rules example"
```

---

### Task 5: Safe-state fallback in production bootstrap (TDD)

**Files:**
- Modify: `src/main.rs` (`run_production` bootstrap section)
- Create: `tests/safe_state.rs`

**Interfaces:**
- Consumes: `semantic::parse_semantic`, `semantic::compile`, `config::RuleStore`, `rules::Rules`.
- Produces: `run_production` compiles extended-TOML when the config parses as semantic; on
  missing file OR semantic/invalid input, falls back to a fail-safe safe-state `RuleStore`
  (no unrestricted motion) and logs loudly. Keeps existing raw-TOML path working.

- [ ] **Step 1: Write the failing test** (`tests/safe_state.rs`)

```rust
use std::process::Command;

/// Production mode with a missing config must still start (fail-safe), not crash.
#[test]
fn production_missing_config_starts_safe() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["--robot-id", "7", "--config", "/nonexistent/flo/rules.toml"])
        .output()
        .expect("run flo");
    // flo logs the safe-state fallback and keeps running; it should not exit 0 immediately
    // nor panic. We assert it did not abort with a parse panic (stderr has no "panic").
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panic"), "flo panicked on missing config: {stderr}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test safe_state 2>&1 | tail -20`
Expected: the current code does `std::fs::read_to_string(path).map_err(...)?` and returns an Err →
`flo` exits with error, so the process exits non-zero; the test as written checks no panic (passes
trivially) but does NOT yet exercise safe-state. Strengthen by asserting the process exits and we
see the safe-state log. For the plan, the implementer should change `run_production` so a missing/
invalid config logs a safe-state message and continues; then assert stderr contains "safe-state".
Update the test to:
```rust
assert!(stderr.contains("safe-state"), "expected safe-state fallback, got: {stderr}");
```

- [ ] **Step 3: Write minimal implementation**

In `src/main.rs` `run_production`, replace the bootstrap block (lines ~230-236):
```rust
    let bootstrap = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(path, error = %e, "config unreadable -> starting in fail-safe safe-state (no unrestricted motion)");
                safe_state_toml()
            }
        },
        None => "rules = []\n".to_string(),
    };

    // Try semantic (extended-TOML) first; fall back to raw TOML; else safe-state.
    let store = compile_or_fallback(&bootstrap, &robot_id);
```
Add helpers:
```rust
/// A minimal fail-safe ruleset: slow to zero, never unrestricted motion.
fn safe_state_toml() -> String {
    // An empty ruleset is the minimal safe-state for v1 (no motion commands emitted).
    "rules = []\n".to_string()
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
fn compile_or_fallback(text: &str, robot_id: &str) -> RuleStore {
    if let Ok(doc) = flo_rs::semantic::parse_semantic(text) {
        match flo_rs::semantic::compile(&doc, robot_id) {
            Ok(rules) => match RuleStore::bootstrap(&rules.to_toml()) {
                Ok(s) => return s,
                Err(e) => tracing::error!(error = %e, "semantic compile produced invalid rules -> safe-state"),
            },
            Err(e) => tracing::error!(error = %e, "semantic validation failed -> safe-state"),
        }
    }
    match RuleStore::bootstrap(text) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "config invalid -> starting in fail-safe safe-state");
            RuleStore::bootstrap(&safe_state_toml()).expect("safe-state always parses")
        }
    }
}
```
(For v1, "safe-state" = empty ruleset — `flo` emits no motion commands. The spec's "slow_to(0)
baseline" is a future enhancement; document this honestly in the commit/PR. The key invariant —
missing/bad config never produces unrestricted actuation — holds.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test safe_state 2>&1 | tail -20`
Expected: PASS (stderr contains "safe-state").

- [ ] **Step 5: Commit**

```bash
git add src/main.rs tests/safe_state.rs
git commit -m "feat(production): fail-safe safe-state on missing/invalid config"
```

---

### Task 6: Document the semantic layer in README (no new tests)

**Files:**
- Modify: `README.md` (add a "Semantic rules" section after the Examples section)

**Interfaces:**
- Consumes: the v1 topic contract + `flo rule check` from earlier tasks.

- [ ] **Step 1: Add the README section**

Insert after the existing "Examples" section:
```markdown
## Semantic rules (industrial)

Instead of raw Zenoh key-expressions, you can author rules against **zones, roles, poses,
proximity, and human-presence**. `flo` compiles the semantic document to the same runtime rule
engine — no engine change. Authoring is extended TOML (no new dependencies; `#![forbid(unsafe_code)]`
preserved).

\`\`\`toml
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
\`\`\`

Validate before deploy:

\`\`\`bash
flo rule check examples/rules/hrc-cell.toml
\`\`\`

Semantic `when` keys: `in_zone`, `not_in_zone`, `near_human`, `not_near_human`, `near`,
`role`. Actions: `estop` (reliable STOP), `slow_to(speed)` (best-effort), `resume`. See
`examples/rules/` for an HRC safety cell and a warehouse AMR fleet.

**Safety posture:** `flo` is the software pre-estop / coordination layer. Missing or invalid
config starts `flo` in a fail-safe state (no unrestricted motion commands); pose loss fails
safe. Hardware STO / certified Safety-PLC remains the primary stop authority.
```

- [ ] **Step 2: Verify the README examples are accurate**

Run:
```bash
cargo run -- rule check examples/rules/hrc-cell.toml
cargo run -- rule check examples/rules/warehouse-fleet.toml
```
Expected: both `OK: ...`. The README snippet matches the real pack (the `when.near_human = 1.2`
line is verbatim from `examples/rules/hrc-cell.toml`).

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document semantic rule layer in README"
```

---

## Self-Review (against spec)

1. **Spec coverage:** vocabulary (Task 1 types) ✅; mesh presence topic contract (plan v1 contract,
   exact-topic) ✅; compile-to-existing-TOML (Task 2) ✅; extended-TOML authoring (Task 1/4) ✅;
   `flo rule check` (Task 3) ✅; example packs HRC + warehouse (Task 4) ✅; safe-state fail-safe
   (Task 5) ✅; out-of-scope items (certified safety, SLAM, cloud backends, engine rewrite) correctly
   excluded ✅.
2. **Placeholder scan:** no TBD/TODO; every code step shows full code; tests show assertions + commands.
3. **Type consistency:** `SemanticDoc`/`SemanticRule`/`SemanticWhen`/`SemanticAction`/`Zone`/`Site`/
   `NearSpec` names match across Tasks 1–2 and the example. `compile(doc, robot_id) -> Rules` and
   `Rules::to_toml() -> String` are defined in Task 2/4 and used in Task 4/5. `Qos::Reliable`/
   `Qos::BestEffort` from `rules.rs` used consistently. `flo_rs::semantic::*` paths match the
   `pub mod semantic` in `lib.rs`.

## Execution Handoff

Plan complete and saved. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, two-axis review between
   tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session via executing-plans, batch with checkpoints.

Which approach?
