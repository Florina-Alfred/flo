# Research: Ruleset / Rule TOML Schema for `flo-engine`

**Date:** 2026-07-20
**Scope:** `flo-engine` rule authoring surface — outer envelope (`ruleset`) wrapping many
`rule` entries, primitive-only payloads, and hot-reload mechanics.
**Verdict:** The existing `rules::Rules` engine needs **no change**. A thin outer
envelope (`Ruleset`) plus a semantic compile step is sufficient.

---

## 1. Current state of the code

### 1.1 Runtime structs (`src/rules.rs`)
- `Qos { Reliable, BestEffort }` — maps to Zenoh class 1 (STOP) / class 2 (lidar).
- `Action { topic, qos, payload: serde_json::Value }` — one publish fired when a rule fires.
- `Trigger { topic, pred: Option<String> }` — key-expression match + optional predicate.
- `When { all: Vec<Trigger>, any: Vec<Trigger> }` — composable AND / OR.
- `Rule { name, when, actions }` — one declarative rule.
- `Rules { rules }` — the flat, anonymous ruleset loaded from TOML today.

### 1.2 Semantic layer (`src/semantic.rs`)
`SemanticWhen` already *extends* `Trigger` with first-class safety primitives:
`in_zone`, `not_in_zone`, `near_human`, `not_near_human`, `near`, `role`, plus nested
`all`/`any`. `compile(doc, robot_id)` lowers a `SemanticDoc` into `rules::Rules`. This
is the right extension point — it is where zone / proximity / human-presence become
runtime `Trigger`s (`fleet/{site}/{robot_id}/state`, `.../proximity/.../human`).

### 1.3 Evaluator (`src/engine.rs`)
`eval_predicate` supports six comparisons (`== != < > <= >=`) over
bool / number / string right-hand sides. It is **primitive-native** — it only ever
reads scalar fields off a `serde_json::Value`. Objects/arrays are unsupported and
would silently fail-open (return `false` on a missing field, or pass on an
unparseable predicate). This is the crux of the "primitive-only" decision.

---

## 2. Recommended TOML shape

A **named, client-owned** envelope wraps the semantic doc. The engine still consumes
`rules::Rules`; only an outer envelope + metadata is added.

```toml
# fleet/{site}/ruleset/{ruleset_name}
ruleset_name = "warehouse-safety-07"
version      = "1.4.2"          # semver string, monotonic
robot_owner  = "fleet/7"        # owning client; only it may re-publish this ruleset

[site]
id    = "wh1"
frame = "map"

[zones.dock]
shape = "rect"
x = 0.0
y = 0.0
w = 4.0
h = 3.0

[zones.aisle]
shape = "rect"
x = 10.0
y = 0.0
w = 2.0
h = 20.0

# ---- one rule ----
[[rule]]
rule_name = "estop-near-human"
when.in_zone = "aisle"
when.near_human = 1.5            # separation_distance < 1.5 m

[[rule.actions]]
estop = true
qos   = "reliable"

# ---- another rule ----
[[rule]]
rule_name = "slow-in-dock"
when.in_zone = "dock"

[[rule.actions]]
slow_to = 0.3
qos     = "best_effort"

# ---- primitive payload echo / publish ----
[[rule]]
rule_name = "publish-state-on-bumper"
when.all = [{ topic = "robot/7/local/bumper", pred = "pressed == true" }]

[[rule.actions]]
topic   = "fleet/wh1/7/incident"
qos     = "reliable"
payload = { pressed = true, at_ms = 1718, label = "bumper", ok = false }
```

### 2.1 Mapping onto existing structs

| TOML key                       | Existing struct / field                         |
|--------------------------------|-----------------------------------------------|
| `ruleset_name`, `version`, `robot_owner` | new `Ruleset` envelope (metadata only)    |
| `[site]`, `[zones.*]`          | `semantic::Site`, `semantic::Zone`            |
| `[[rule]]`                     | `semantic::SemanticRule` → `rules::Rule`      |
| `rule_name`                    | `Rule.name`                                    |
| `when.*`                       | `semantic::SemanticWhen` → `When` / `Trigger` |
| `[[rule.actions]]`             | `semantic::SemanticAction` → `rules::Action`  |
| `payload = { … }`             | `Action.payload: serde_json::Value`           |

The envelope deserializes to:

```rust
#[derive(Deserialize)]
pub struct Ruleset {
    pub ruleset_name: String,
    pub version: String,
    pub robot_owner: String,
    #[serde(default)]
    pub site: Site,
    #[serde(default)]
    pub zones: HashMap<String, Zone>,
    #[serde(default)]
    pub rule: Vec<SemanticRule>,   // was `rules` in SemanticDoc
}
```

No field on `rules::Rules`, `When`, `Trigger`, or the evaluator changes.

---

## 3. Primitive-only payloads

`eval_predicate` only compares **scalar** fields. Therefore:

- **Allowed payload types:** `bool`, `int`, `float`, `string` (the five "primitives"
  including the JSON number split as int/float). These serialize cleanly to
  `serde_json::Value` and are comparable by `cmp` / `==`.
- **Author-time validation:** reject any `payload` (or trigger `pred` RHS) that is an
  object/array. Add a `validate` pass in the semantic layer that walks every
  `Action.payload` and returns `SemanticError` on non-primitives. This keeps the
  engine's fail-open behaviour from ever being reached for malformed config.
- **Complex payloads stay the publisher's concern.** A rule action may still publish a
  primitive summary (e.g. `speed_mps`, `pressed`) to a command topic; rich/structured
  telemetry remains the originating publisher's responsibility and is not parsed by the
  engine. This bounds engine complexity and preserves the auditable, declarative shape.

---

## 4. Hot-reload mechanics

Owning client re-publishes the **entire** ruleset to the envelope topic
`fleet/{site}/ruleset/{ruleset_name}` (whole-doc swap, not patch deltas — simpler and
atomic):

1. **Receive** new `Ruleset` TOML on the envelope topic (subscribed once per `ruleset_name`).
2. **Parse + validate** (`Ruleset::from_toml` → `SemanticDoc`/`Ruleset` validate →
   `compile`). Reject non-primitive payloads here.
3. **Version gate** — refuse to apply if `version` is not strictly greater than the
   currently active ruleset (monotonic, prevents replay/stale overwrite).
4. **Atomic swap** — `RuleStore::current()` is an `Arc<Rules>` swap. Build the new
   `Rules` fully *before* swapping so the live engine never observes a partial state.
5. **Keep last-good** — if validation or compile fails, **discard the new doc and keep
   the previous `Rules` active**; log + emit an audit event. The engine never goes
   "rules-less."
6. **Audit copy + SHA** — on successful swap, persist the accepted TOML bytes plus
   `sha256(ruleset || version || robot_owner)` to an append-only audit log
   (see `research/audit-sha`). This gives a tamper-evident history for compliance.

The engine's re-eval loop already calls `store.current().await` every tick, so a
swapped `Arc` is picked up within one 50 ms tick with zero restart.

---

## 5. Safety-config precedent (survey)

| Source | Relevant precedent |
|--------|--------------------|
| **ROS 2 parameters / launch** | Declarative, namespaced param trees; `launch` describes node graph + params. Our `ruleset` envelope mirrors the "named, versioned, overridable config" idiom; `robot_owner` mirrors ROS 2 node namespaces/ownership. |
| **Behavior Trees (BT)** | Composable, auditable decision graphs; tick-based re-eval. Our `When{all,any}` + 50 ms re-eval loop is a degenerate, data-only BT — declarative, replayable, human-readable. |
| **Semantic safety filters** | Zone / proximity / human-presence as *first-class* conditions. `SemanticWhen` already promotes these; they belong in the schema, not buried in predicate strings. |
| **ISO 3691-4 (industrial trucks)** | Requires speed limitation, stop, and separation monitoring in defined zones; behaviour must be deterministic and verifiable. Zone + `near_human` primitives directly encode these requirements. |
| **ANSI B56.5 (automated guided vehicles)** | Mandates detection of pedestrians / obstacles and defined protective fields; auditing of safety behaviour. The audit-copy + SHA and named versioned ruleset satisfy the "demonstrable, recorded safety logic" requirement. |

**Takeaway:** a named, versioned, auditable, primitive-scoped ruleset with
zone/proximity/human-presence as first-class primitives is consistent with every major
industrial-safety and robotics-config precedent.

---

## 6. Open questions

- Should `robot_owner` be a single client id or an ACL list? (Single owner keeps the
  hot-reload authorization trivial; multiple owners need signing.)
- Does `version` live as semver or a monotonically increasing `u64` counter? semver is
  human-friendly; a counter is replay-proof. Recommend semver + SHA gating.
- Where does the audit log physically live (zenoh durable topic vs. local file vs.
  object store)? See `research/audit-sha`.
