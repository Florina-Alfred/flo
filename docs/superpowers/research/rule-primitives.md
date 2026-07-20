# Research: Five Rule Primitives & Predicate Grammar

**Ticket:** #66 — "Five rule primitives & predicate grammar (site/zone/robot/proximity/human_presence)"
**Repo:** `flo-rs` (single binary `flo`, Rust, Zenoh transport)
**Branch:** `research/rule-primitives`
**Status:** Research / proposal — extends `src/rules.rs` `When`/`Trigger` model.

---

## 1. Problem & Scope

`flo` evaluates declarative rules against a Zenoh pub/sub stream. Today a rule is:

```rust
struct Trigger { topic: String, pred: Option<String> }   // src/rules.rs:26
struct When    { all: Vec<Trigger>, any: Vec<Trigger> }   // src/rules.rs:38
struct Rule    { name: String, when: When, actions: Vec<Action> }
```

The `pred` field is an **unevaluated string** — there is no grammar, no type
system, no safety story. This document defines five first-class **primitives**
a rule can reference and a **deterministic, auditable predicate grammar** over
their primitive (bool/float/int/string) state. The grammar is intentionally
*not* Turing-complete: no loops, no functions, no arithmetic on arbitrary
expressions — only comparisons and boolean combination. That is the property
that lets a rule be statically validated, unit-tested, and audited against a
safety standard.

---

## 2. The Five Primitives

Each primitive is **a stream of observations published on a Zenoh key-expression**
plus an **observed value** of a primitive type. A rule never reaches into a
robot or sensor directly; it subscribes to the published event/state and reads
the typed value.

| Primitive | Source topic(s) | Observed value type | Carries |
|---|---|---|---|
| `site` | `/<site>/entered`, `/<site>/exited` | event + `{robot:id}` | A robot entered/exited a named site (geofence super-set). |
| `zone` | `/<zone>/human_present`, `/<zone>/human_cleared` | event + `{human:id}` | A human present/cleared in a named zone (geofence sub-region). |
| `robot` | `/robot/<id>/**` | state stream (pose, speed, proximity, mode) | Per-robot live state, **auto-subscribed** by any client that names this robot in a rule. |
| `proximity` | `/robot/<id>/**` (pose/proximity signal) | float (meters) | Separation distance from this robot to the nearest human/obstacle. |
| `human_presence` | `/<zone>/human_present` or `/<site>/entered` | bool/event | Boolean "is a human present" derived from zone/site topics. |

### 2.1 `site` / `zone` — geofence events

Safety field robotics partitions space into nested geofences. Mirrors
**ISO 3691-4:2023** operating-zone model (operating zones with personnel,
restricted zones, confined zones — §5, Annex A) and the **ISO/TS 15066**
fixed/variable *safety zones* (green/yellow/red) used for SSM.

A site is a named super-region; a zone is a named sub-region. Both publish
**edge events** when membership changes:

```
/site_a/entered     { "robot": "7" }
/site_a/exited      { "robot": "7" }
/zone_1/human_present { "human": "h12" }
/zone_1/human_cleared { "human": "h12" }
```

The observed value a rule reads is **the event itself** plus its payload fields
(`robot`, `human`, etc.) as strings. Matching is by **Zenoh key-expression**
(see §6) — `site`/`zone` rules are *event subscriptions*, not state polling.

### 2.2 `robot` — auto-subscribed state stream

Per **ISO 10218-1:2011 §5.10.4 / ISO/TS 15066 §5.5.4** SSM, the safety system
must continuously know each robot's position and velocity. A client that names
`robot/7` in any rule **auto-subscribes to `/robot/7/**`** — no manual
subscription wiring. All per-robot predicates (`proximity`, speed, pose, mode)
read fields from this single stream, which keeps the rule's data dependencies
explicit and bounded (a finite, declared topic set).

### 2.3 `proximity` — separation distance (float)

`proximity(robot_2)` is the instantaneous separation distance from robot_2 to
the nearest human/obstacle, in meters, carried in the `/robot/<id>/**` stream
(pose/proximity signal). This is the core SSM quantity:

> *"When the separation distance decreases to a value below the protective
> separation distance, the robot system stops."* — ISO/TS 15066 §5.5.4.1

The protective separation distance `S` (ISO/TS 15066 eq., ISO 13855 intrusion
margin `C`) is itself computed by the *robot firmware*, not by `flo` rules.
`flo` rules consume the **already-safety-rated `proximity` float** and gate
higher-level behavior (announce, slow, stop) on thresholds. This separation of
concerns is what keeps `flo` rules auditable: they compare a primitive float
against a constant.

### 2.4 `human_presence` — boolean/event

Boolean derived from `zone`/`site` human topics. In standards terms this is the
**personnel-detection-system (PDS)** output of ISO 3691-4 §5.7, and the
presence/safe-monitored-stop trigger of ISO/TS 15066 §5.5.2. `flo` treats it as
a primitive `bool` (or an event) so rules can say `human_presence(zone_1) ==
true` without re-deriving detection.

---

## 3. Primitive-Type Constraints (the "stay primitive" rule)

Observed values are restricted to four scalar kinds. **No compound, no
reference, no side-effecting value.** This is what makes evaluation
deterministic and unit-testable.

| Type | Examples | Comparison operators allowed |
|---|---|---|
| `bool` | `human_presence`, `e_stop` | `==`, `!=` |
| `float` | `proximity`, `speed` | `==`, `!=`, `<`, `>`, `<=`, `>=` (with epsilon for `==`) |
| `int` | `robot_id`, `battery_pct` | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| `string` | `site`, `zone`, `human_id`, `mode` | `==`, `!=` |

- Floats are compared with an explicit epsilon for equality (configured per
  ruleset, default `1e-3`) to avoid NaN/ULP surprises — a deterministic
  requirement for audit.
- All literals in a rule are constants; a rule cannot read another rule's
  output. Cyclic/derived state is forbidden → **no fixpoint ambiguity**.
- Every predicate evaluates in O(1) against the current observed value; there is
  no time-window or history operator (keeps it stateless and replayable for
  audit). Time-based rules (dwell, rate) are a deliberate future extension and
  must not be expressible today.

---

## 4. Predicate / Operator Grammar (BNF)

The grammar extends `When`/`Trigger`. A `Trigger` today matches a topic and an
optional free-text `pred`. We replace the free text with a **typed predicate
tree**, keeping `When { all, any }` as the boolean composition layer.

```ebnf
# A rule's guard (unchanged shape, richer Trigger)
When        ::= "{" "all" ":" "[" Trigger* "]"
                  "any" ":" "[" Trigger* "]" "}"

# A Trigger now names a primitive + a predicate over it
Trigger     ::= "{" "on"   ":" PrimitiveRef
                  "when" ":" Predicate "}"      # "when" optional => match-only

PrimitiveRef::= SiteRef | ZoneRef | RobotRef | ProximityRef | PresenceRef

SiteRef     ::= "site"   "(" string ")"          # e.g. site("site_a")
ZoneRef     ::= "zone"   "(" string ")"          # e.g. zone("zone_1")
RobotRef    ::= "robot"  "(" int ")"             # e.g. robot(7)
ProximityRef::= "proximity" "(" int ")"          # e.g. proximity(7)
PresenceRef ::= "human_presence" "(" string ")"  # zone or site name

# Predicate over a primitive's observed value
Predicate   ::= Comparison
              | "(" Predicate ")"
              | Predicate "and" Predicate
              | Predicate "or"  Predicate
              | "not" Predicate

Comparison  ::= Operand Op Operand
Operand     ::= PrimitiveValue | Literal
PrimitiveValue ::= SiteRef | ZoneRef | RobotRef | ProximityRef | PresenceRef
Op          ::= "==" | "!=" | "<" | ">" | "<=" | ">="
              | "same_zone_as"          # robot-robot spatial relation
Literal     ::= bool | float | int | string
```

### 4.1 Operators

| Operator | Domain | Meaning |
|---|---|---|
| `==`, `!=` | all types | equality / inequality (float via epsilon) |
| `<`, `>`, `<=`, `>=` | float, int | ordered comparison |
| `and`, `or`, `not` | predicate | boolean composition (matches `When.all`/`any` semantics) |
| `same_zone_as` | robot × robot | true iff both robots' latest pose falls in the same named zone (derived from `robot` pose + zone geometry). Spatial relation operator — see §4.2. |

`same_zone_as` is the one non-trivial operator. It is **not** arbitrary
geometry: it evaluates to a bool by looking up each robot's current zone
membership (a string) and comparing for equality. It reduces to `string ==`,
so it stays within the primitive-type system and remains deterministic.

### 4.2 Reserved / explicitly out-of-scope operators

To keep the grammar non-Turing-complete and auditable, the following are
**forbidden** and the parser must reject them:

- arithmetic (`+ - * /`), functions, variables, assignments
- quantifiers / loops (`for`, `exists`)
- time-window / history operators (`within`, `after`, `count`)
- inequality chains, implication

This matches the safety literature's demand for *deterministic, verifiable*
guards (ISO 13849 PL d, IEC 61508 SIL 2 classes cited for SSM in
ANSI/RIA R15.08 / ISO 10218-2).

---

## 5. Mapping to `When` / `Trigger` (concrete examples)

The grammar maps directly onto the existing structs. `Trigger.on` becomes the
topic/primitive reference; `Trigger.when` becomes the typed `Predicate` (the
old `pred: Option<String>` is replaced by `pred: Option<Predicate>` or a
parsed AST). `When.all`/`When.any` already give AND/OR.

```toml
# Stop robot 7 if a human is present in zone_1 AND proximity(7) < 1.2 m
[[rules]]
name = "zone1_proximity_stop"
when.all = [
  { on = "human_presence(zone_1)", when = "== true" },
  { on = "proximity(7)",           when = "< 1.2" },
]
actions = [ { topic = "stop/fleet/cmd", qos = "reliable",
             payload = { robot = 7, reason = "proximity" } } ]

# Robot 7 must not enter site_a while robot 8 is in the same zone
[[rules]]
name = "no_same_zone_entry"
when.all = [
  { on = "site(\"site_a\")", when = "entered robot == 7" },
  { on = "robot(7)", when = "same_zone_as robot(8)" },
]
actions = [ { topic = "robot/7/local/drive", qos = "reliable",
             payload = { cmd = "hold" } } ]

# Slow robot 3 whenever any human present in its operating zone
[[rules]]
name = "pds_slow"
when.any = [
  { on = "human_presence(zone_3)", when = "== true" },
]
actions = [ { topic = "robot/3/local/speed", qos = "best_effort",
             payload = { limit_mps = 0.3 } } ]   # ISO 3691-4 §5.6 docking limit
```

Note the `entered robot == 7` form: site/zone event payloads expose their
fields (`robot`, `human`) as strings, so a `SiteRef`/`ZoneRef` predicate
compares a **payload field** rather than the primitive value itself. We extend
`Operand` to allow `PrimitiveValue "." field`, i.e. `site("site_a").robot ==
"7"`. This keeps event-matching inside the same typed grammar.

---

## 6. Zenoh Key-Expression Matching (transport layer)

All topic strings are **Zenoh key-expressions** (spec.zenoh.io, Key Expressions
1.0). The `on` reference compiles to a KE the client subscribes to:

| Primitive ref | Compiled key-expression | Match semantics |
|---|---|---|
| `site("site_a")` | `/site_a/**` | event + payload filter |
| `zone("zone_1")` | `/zone_1/**` | event + payload filter |
| `robot(7)` | `/robot/7/**` | auto-subscribe; full state stream |
| `proximity(7)` | `/robot/7/**` (pose/proximity field) | reads `proximity` field of stream |
| `human_presence("zone_1")` | `/zone_1/human_*` | event/boolean |

Zenoh wildcards: `*` = exactly one chunk, `**` = zero-or-more chunks. KEs have a
**canonical form** and the *unicity property* (two KEs address the same key set
iff they are the same string) — this gives us a cheap, exact way to (a) detect
when two rules reference the same stream and (b) prove a client subscribes to
exactly the union of its rules' KEs (auditable subscription set). `robot(id)`
auto-subscription is simply: `subscribe(union_of_all KEs where ref is robot/id)`.

---

## 7. Safety-Standards Precedent

| Standard | Concept reused | How it informs the grammar |
|---|---|---|
| **ISO 3691-4:2023** (driverless industrial trucks / AMR) | Operating zones with personnel; stopping distance = f(speed, load); PDS; 0.3 m/s muted-detection docking limit (§5.6, §5.7, Annex A) | `zone`/`site` geofence events; `human_presence` = PDS output; `proximity` float; constant thresholds (no arithmetic in rules). |
| **ISO 10218-1:2011 §5.10.4 / ISO/TS 15066:2016 §5.5.4** (SSM) | Protective separation distance `S`; stop when separation `< S`; variable/fixed zones | `proximity(robot) < const` is the direct expression of SSM; `same_zone_as` encodes zone membership. |
| **ISO/TS 15066 §5.5.2** (safety-rated monitored stop) | Stop on human presence, resume only on deliberate restart | `human_presence == true` → stop action; rules are stateless (no auto-resume logic baked in). |
| **ISO 13855** (positioning of safeguards) | Intrusion margin `C`; detection capability `d` | Justifies float-epsilon equality and treating `proximity` as already-safety-rated input. |
| **ISO 13849 (PL d) / IEC 61508 (SIL 2)** | Deterministic, verifiable safety functions | Grammar is non-Turing-complete, stateless, O(1), replayable → statically auditable. |
| **ANSI/RIA R15.08 / R15.06** | US harmonized equivalent of ISO 10218 | Same SSM/zone model; confirms geofence + proximity guard pattern is the field norm. |
| **ROS 2 nav2 / costmap** | Costmap layers mark lethal/inflation around humans; "keepout" and "speed-restricted" zones | Confirms zone-typed abstraction + distance-threshold guards are the established robotics idiom `flo` should mirror. |

**Fitness for safety-critical use.** The grammar is:
1. **Deterministic** — no loops, no functions, no time-dependence; one evaluation = one boolean.
2. **Auditable** — every rule is a finite tree of typed comparisons; it can be printed, diffed, and proven to reference only declared KEs.
3. **Non-Turing-complete** — cannot express unbounded computation, so worst-case eval time is bounded and reviewable.
4. **Primitive-typed** — values are bool/float/int/string only; no opaque objects, no hidden state.
5. **Subscription-explicit** — a client subscribes only to the union of KEs its rules name (via Zenoh unicity), so the data flow is closed and reviewable.

This satisfies the verification posture expected of PL d / SIL 2 safety
functions while staying a pure configuration language.

---

## 8. Open Questions for Implementation

1. **TOML vs dedicated DSL?** The grammar parses from TOML today (per `Rules::from_toml`). Recommend keeping TOML as the serialization and adding a
   `Predicate` AST type + `from_str` parser; reject unknown operators at load
   (`from_toml` already returns `Result` so bad config keeps the prior ruleset).
2. **Event payload field access** (`site("a").robot == "7"`) — confirm payload
   schema is stabilized before shipping field comparison.
3. **`same_zone_as` provenance** — zone membership may be computed by an
   external geofencer; `flo` should consume a `zone` field on the robot stream
   rather than do geometry. Recommend that.
4. **Time/dwell rules** — explicitly deferred; add only with a bounded,
   statically-sized window operator if ever needed.

---

*Sources: ISO 3691-4:2023; ISO 10218-1:2011 & ISO/TS 15066:2016 (SSM, §5.5.4);
ISO 13855; ISO 13849-1; IEC 61508; ANSI/RIA R15.08; Zenoh Key-Expression
Specification 1.0 (spec.zenoh.io); ROS 2 nav2 costmap convention.*
