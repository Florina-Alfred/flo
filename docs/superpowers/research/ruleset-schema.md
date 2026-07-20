# Research: Ruleset & Rule TOML Schema

**Ticket:** #65 — Ruleset & rule TOML schema (`ruleset_name`, `rule_name`, hot-reload, primitive payloads)
**Depends on:** #66 (five primitives + predicate grammar), #68 (collision policy + audit/SHA)
**Scope:** Schema *shape* only here; predicate grammar and SHA granularity are cross-referenced, not finalized.
**Status:** Research / recommendation.

---

## 1. Problem statement

`flo-engine` (`src/rules.rs`) today has a flat, unnamed rule list:

```rust
pub struct Rules { pub rules: Vec<Rule> }
pub struct Rule  { pub name: String, pub when: When, pub actions: Vec<Action> }
pub struct When  { pub all: Vec<Trigger>, pub any: Vec<Trigger> }
pub struct Trigger { pub topic: String, pub pred: Option<String> }
```

The runtime `src/semantic.rs` already extends this with a `SemanticDoc` (`site`, `zones`, `rules` of `SemanticWhen`/`SemanticAction`) and compiles down to `Rules`. But there is **no `ruleset_name`**, no per-rule `rule_name` distinct from the flat `Rule.name`, no hot-reload ownership model, and no statement that payloads are **primitive-only**.

This research proposes the TOML shape: a named ruleset owning many rules, each rule carrying its own name, expressing the five primitives (`site`/`zone`/`robot`/`proximity`/`human_presence`) over primitive payloads, and being hot-reloadable with a server audit copy.

---

## 2. State-of-the-art survey

### 2.1 ROS 2 parameter & launch files
ROS 2 configures nodes via YAML parameter files (`ros__parameters:` keyed by node, typed `bool/int/double/string/array`) and launch files (YAML/XML/Python) that set parameters, namespaces, and remappings for *whole deployments* in one place. Key takeaways for `flo`:
- **One outer object wraps many named entries** (a node = a ruleset; its parameters = rules).
- **Namespacing** (`/robot/<id>/...`) is first-class — confirmed by `semantic.rs` already emitting `fleet/{site}/{robot_id}/state`.
- **YAML/JSON Schema validation** is now standard tooling (the `ros2_awesome` JSON Schema, VS Code completion). We should ship a TOML/JSON schema for the ruleset so misconfig fails *before* hot-reload.
- ROS 2 dynamic reconfigure already proves **runtime parameter mutation without restart** is an expected pattern → hot-reload is precedented.

### 2.2 Behavior Trees (BT) as the safety-decision precedent
Open-source safety-critical stacks (e.g. `robot_safety_decision_system`, Colledanchise & Ögren BTs) encode safety as:
- **Small, testable condition/action nodes**, composed into a tree, **re-ticked every cycle** (reactivity).
- A **ReactiveSequence / ReactiveFallback** structure where safety conditions are continuously re-evaluated and can *preempt* long-running actions (a hard e-stop beats navigation).
- Groot2 visualization for **debugging/audit of decisions**.

Relevance to `flo`: our `When{all,any}` is the composable boolean layer that mirrors BT guard conditions. The schema should keep rules **composable, declarative, and re-evaluable** (engine already re-ticks every 50 ms in `engine.rs:142`). We deliberately stay at the *rule* layer, not the BT layer — BTs are the consumer's orchestration choice; `flo` ships deterministic triggers.

### 2.3 Semantic / safety rule layers
- `semantic.rs` is already a **semantic authoring layer**: authors write `in_zone` / `near_human` / `role` and it compiles to low-level `Trigger{topic,pred}` tuples. This is exactly the "semantic constraint → guard condition" mapping seen in *Semantically Safe Robot Manipulation* (2025) and construction-robot WRC work ("encode dynamic safety rules as guard conditions within a semantic workflow model").
- The pattern "**rules propose, envelope disposes**" (non-overridable safety envelope, seen in recent ROS 2 HRI projects) recommends: authored rules are *proposals*; a deterministic, non-overridable engine (our `When` evaluator, `engine.rs`) has final say. The schema must therefore be **fail-safe / fail-open explicit** — see §5.

### 2.4 Functional-safety standards (ISO 3691-4, ANSI/ITSDF B56.5)
- **ISO 3691-4:2020 + Amd 1:2023** (driverless industrial trucks) requires: formal risk assessment (ISO 12100), **performance level PLd** for safety functions, personnel detection stopping *before* contact, speed limiting tied to protective-field size, and **validation of the complete system**.
- **ANSI/ITSDF B56.5-2019**: personnel detection of a 200 mm proxy, automatic stop on sensor obstruction, max speed from stopping-distance math, documented risk assessments, annual inspection.
- Both mandate **auditable, documented safety behavior** and zone/path definitions.

Relevance: a ruleset *is* the documented, auditable safety behavior. Hence (#68) the server keeps an audit copy + SHA; this maps directly onto "documented risk assessment / inspection record" requirements. The five primitives (`zone`, `proximity`, `human_presence`) are the schema-level expression of 3691-4's protective-field / personnel-detection concepts.

---

## 3. Recommended TOML shape

A single `ruleset` is the unit of ownership, hot-reload, and audit. It wraps many `rule`s. Each rule keeps its own `rule_name`. Primitives are expressed via `when` (semantic-aware, mirroring `SemanticWhen`) and payloads are **primitive-only**.

```toml
# ruleset is the top-level, server-unique, client-owned object.
ruleset_name = "acme-site-a-forklift-fleet"   # unique on server (collision => reject, per #68)
version = 3                                   # monotonic; bump on every client edit
robot_owner = "robot/7"                       # client that owns & re-publishes this ruleset

[site]
id = "site_a"
frame = "map"

[zones.zone_1]
shape = "rect"
x = 0.0
y = 0.0
w = 5.0
h = 3.0

[zones.zone_2]
shape = "circle"
x = 10.0
y = 4.0
r = 2.0

# ---- Rule 1: proximity to a human -> slow down -----------------------------
[[rule]]
rule_name = "slow_near_human"                 # per-rule name, distinct from ruleset_name
# `when` extends When/Trigger with the five primitives (see #66 for grammar):
when.site   = "site_a"                        # primitive: string
when.in_zone = "zone_1"                       # primitive: string
when.near_human = 1.2                         # primitive: float (metres)
when.role   = "forklift"                      # primitive: string

[[rule.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { speed_mps = 0.3 }                # PRIMITIVE payload only (float)

# ---- Rule 2: human present in zone_2 -> e-stop -----------------------------
[[rule]]
rule_name = "estop_human_in_zone2"
when.in_zone = "zone_2"
when.human_presence = true                    # primitive: bool (event/state flag)

[[rule.actions]]
topic = "stop/fleet/cmd"
qos = "reliable"
payload = { stop = true }                     # PRIMITIVE payload only (bool)

# ---- Rule 3: composite AND/OR over primitives -----------------------------
[[rule]]
rule_name = "keep_clear_of_peer"
when.robot = "robot/12"                       # primitive subject: another robot
when.proximity = { to = "robot/12", dist = 2.0 }  # primitive: float separation
when.any = [                                  # OR-branch (extends When.any)
  { human_presence = true },
  { in_zone = "zone_1" },
]

[[rule.actions]]
topic = "robot/7/local/drive"
qos = "best_effort"
payload = { speed_mps = 0.0 }
```

### Mapping onto existing structs

| TOML | Existing struct | Notes |
|---|---|---|
| `ruleset_name`, `version`, `robot_owner` | *(new on `Rules`/wrapper)* | Ownership + audit key (#68). `Rules` gains an outer `Ruleset` envelope. |
| `site.id`, `site.frame` | `SemanticDoc.site` | Already exists; keep. |
| `zones.*` | `SemanticDoc.zones` | Already exists; keep. |
| `[[rule]]` | `Rule` | Rename `Rule.name` → `rule_name`; the flat `Rule` becomes a member of the ruleset envelope. |
| `rule.when.*` | `SemanticWhen` | The five primitives (`site`/`zone`/`robot`/`proximity`/`human_presence`) extend `SemanticWhen`; compile to `When{all,any}` of `Trigger{topic,pred}` as `semantic.rs::compile` already does. |
| `rule.actions[].payload` | `Action.payload: serde_json::Value` | **Constrained to primitive JSON** (bool/number/string) by validation — complex payloads are the publisher's own concern, not `flo`'s. |

Concretely:

```rust
#[derive(Deserialize, Serialize)]
pub struct Ruleset {
    pub ruleset_name: String,
    #[serde(default)] pub version: u64,
    #[serde(default)] pub robot_owner: Option<String>,
    #[serde(default)] pub site: Site,
    #[serde(default)] pub zones: HashMap<String, Zone>,
    pub rule: Vec<RuleEntry>,   // was Rules.rules: Vec<Rule>
}

#[derive(Deserialize, Serialize)]
pub struct RuleEntry {
    pub rule_name: String,      // was Rule.name
    #[serde(default)] pub when: SemanticWhen,  // extended with 5 primitives
    pub actions: Vec<SemanticAction>,
}
```

`Rules::from_toml` / `compile` stay intact; we wrap them. `engine.rs` keeps firing `Action`s — no engine change needed for the envelope.

---

## 4. Hot-reload mechanics

Model (client-owned, per ticket):

1. **Client owns the ruleset.** The owning robot (`robot_owner`) is the only writer. It edits TOML locally and **re-publishes** the whole ruleset to a well-known key-expression, e.g. `fleet/{site}/ruleset/{ruleset_name}`.
2. **Server subscribes** to `fleet/+/ruleset/+`. On a new message it:
   - Computes a **SHA** of the canonical TOML (per #68 — recommend **per-ruleset SHA** for the envelope plus an optional per-`rule_name` SHA for forensic granularity).
   - Checks `ruleset_name` collision: same owner re-push = **update**; different owner = **reject with conflict** (#68a).
   - Validates against the schema; on failure it **keeps the last-good ruleset active** (fail-safe, never blank the fleet).
   - Stores an **audit copy** (immutable, version-stamped) for replay / compliance / forensic use (#68b).
3. **Engine swap.** `RuleStore::current()` (used every 50 ms tick in `engine.rs:148`) atomically swaps the active `Rules`. Subscriptions to sensor topics are recomputed when the trigger-topic set changes. No restart.
4. **Primitive payloads** mean re-evaluation is cheap and deterministic — the newest sample per topic wins; no schema/version coupling between producer and engine.

This matches ROS 2 dynamic reconfigure (runtime mutation, no restart) and BT re-tick reactivity.

---

## 5. Primitive-payloads principle

- Observed topic payloads are **bool / int / float / string only**. The predicate evaluator in `engine.rs::eval_predicate` already supports exactly these four (`Value::Number/String/Bool`), so the constraint is native, not bolted on.
- Complex payloads (point clouds, meshes, telemetry structs) are the **publishing client's concern** — `flo` only sees the primitive projection the rule predicates on (e.g. `separation_distance < 1.2`, `human_present == true`).
- Validation must reject non-primitive `payload` values at compile time (extend `semantic.rs::validate`) so a bad ruleset fails closed at authoring, not at fire time.

---

## 6. Safety-config precedent summary

| Source | Principle adopted into schema |
|---|---|
| ROS 2 params/launch | One named wrapper object; namespaced topics; schema-validated config; runtime reconfigure = hot-reload. |
| Behavior Trees (ISO-adjacent safety stacks) | Composable `when.all`/`when.any` guards, continuous re-tick, rules-propose/envelope-disposes (fail-safe). |
| Semantic safety filters (2025) | Author in semantic primitives; compile to low-level triggers; guard-condition model. |
| ISO 3691-4 / B56.5 | Auditable documented behavior → server audit copy + SHA; zone/proximity/human_presence as first-class primitives; PLd-equivalent determinism via non-overridable engine. |

---

## 7. Open questions → other tickets

- **#66** finalizes the predicate grammar (`same_zone_as`, `proximity(robot) < d`, operator set). Schema *shape* above is grammar-agnostic; only the `when.*` field names are fixed here.
- **#68** finalizes collision policy (confirmed: reject-on-conflict) and SHA granularity (recommend per-ruleset + optional per-rule). The `version` + `robot_owner` fields above are the hooks it needs.
