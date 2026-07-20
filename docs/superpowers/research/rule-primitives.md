# Research: Five Rule Primitives & Predicate Grammar

**Repo:** `flo` (Rust crate `flo-rs`, single binary `flo`)
**Scope:** Extend `When`/`Trigger` in `src/rules.rs` with five rule primitives and a typed, auditable predicate grammar.
**Status:** Research only — no code changes. Candidate design for an RFC/PRD.
**Branch:** `research/rule-primitives`

---

## 1. Motivation

Today `src/rules.rs` defines:

- `When { all: Vec<Trigger>, any: Vec<Trigger> }` — composable AND/OR over triggers.
- `Trigger { topic: String, pred: Option<String> }` — a key-expression match plus a **free-text** predicate evaluated against the payload.

The `pred: Option<String>` field is the weak point: it is unimplemented (treated as "always true"), unevaluable, and unauditable. For a safety-rated fleet controller we need predicates that are:

- **Deterministic & O(1)** — no loops, no arithmetic, no closures, no time.
- **Auditable** — every subscribed topic and every comparison is statically enumerable (maps to PL d / SIL 2 expectations).
- **Typed against primitive observations** — only `bool` / `float` / `int` / `string` cross the boundary.

This doc specifies five primitives and a predicate grammar that replaces `pred`.

---

## 2. Five Rule Primitives

All observed values are **primitive** (`bool` | `float` | `int` | `string`). No structured/opaque payloads leak into predicates.

### 2.1 `site`

A **site** is a coarse geographic envelope (a building, a bay, a dock). The fleet publishes edge events on well-known key-expressions:

- `/<site>/entered { robot: <id> }` — a robot entered the site.
- `/<site>/exited  { robot: <id> }` — a robot left the site.

`site` matches the `<site>` segment. It is the highest-level spatial gate (e.g. "only act when inside `site_a`").

### 2.2 `zone`

A **zone** is a finer sub-region of a site (aisle, human-walkway, pick-cell). Edge events:

- `/<site>/<zone>/entered      { robot: <id> }`
- `/<site>/<zone>/exited       { robot: <id> }`
- `/<site>/<zone>/human_present { human: <id> }` — a human is present in the zone.

`zone` matches a `<zone>` segment within a site. Zones are the unit of spatial coordination behind ISO 3691-4:2023 protected areas / PDS envelopes.

### 2.3 `robot`

`robot` auto-subscribes to the entire subtree `/robot/<id>/**`. This is the robot's own telemetry/safety stream: bumper, e-stop, drive-state, battery, localization. A `robot` primitive references a specific `robot_id` and exposes its published primitives by name.

Because the subscription is a single key-expression (`/robot/<id>/**`), the full auditable subscription set is enumerable from the ruleset — no hidden topics.

### 2.4 `proximity(robot_id)`

`proximity(robot_id)` reads the **safety-rated separation distance** (a `float`, metres) published on the robot's stream — e.g. `/robot/<id>/safety/separation`. This is the measured clearance to the nearest detected human/obstacle under the Safety Sensor Module (SSM).

It is consumed only inside comparisons (`proximity(r) < 0.5`) so an evaluator can treat it as a `float` primitive. It must come from a safety-rated source (ISO/TS 15066 §5.5.4, ISO 13855).

### 2.5 `human_presence`

`human_presence` is a `bool`/`event` derived from `human_present` events on zone/site topics (`/zone_1/human_present {human}`). It is the canonical "is a human in this volume right now" predicate input, backed by the zone's presence detection.

| Primitive | Source key-expression | Observation type | Example |
|-----------|----------------------|------------------|---------|
| `site`    | `/<site>/entered`    | event `{robot}`  | `site == "site_a"` |
| `zone`    | `/<site>/<zone>/entered`, `/.../human_present` | event `{robot}`/`{human}` | `zone == "zone_1"` |
| `robot`   | `/robot/<id>/**`     | stream of primitives | `robot(7).bumper == true` |
| `proximity(robot_id)` | `/robot/<id>/safety/separation` | `float` | `proximity(7) < 0.5` |
| `human_presence` | `/<site>/<zone>/human_present` | `bool`/`event` | `human_presence == true` |

---

## 3. Predicate Grammar

### 3.1 Type constraints

| Type | Literals / values | Operators |
|------|-------------------|-----------|
| `bool` | `true`, `false` | `==`, `!=` |
| `float` | decimal, e.g. `0.3`, `1.2` | `== != < > <= >=` |
| `int` | integer, e.g. `7` | `== != < > <= >=` |
| `string` | `"site_a"`, `"zone_1"` | `== !=`, `same_zone_as` |

`proximity(...)` and separation floats are `float`. `human_presence` is `bool`. Equality/ordering only; **no arithmetic, no string concat, no function calls except `proximity(id)`**, no time, no loops.

### 3.2 BNF-ish grammar

```
Predicate      ::= Comparison (BoolOp Comparison)*
BoolOp         ::= "and" | "or"
Comparison     ::= Operand Op Operand
                |  "not" Comparison
                |  "(" Predicate ")"
Operand        ::= PrimitiveRef
                |  Literal
PrimitiveRef   ::= "site"
                |  "zone"
                |  "human_presence"
                |  "robot" "(" Int ")" "." Field
                |  "proximity" "(" Int ")"
Field          ::= Identifier            ; a named primitive on the robot stream
Literal        ::= BoolLit | FloatLit | IntLit | StringLit
Op             ::= "==" | "!=" | "<" | ">" | "<=" | ">="
                |  "same_zone_as"        ; string-only spatial equivalence
```

Notes:

- `same_zone_as` is a string operator: `robot(7).zone same_zone_as robot(3).zone` — true when both robots' current zone fields resolve to the same zone id. It is the primitive equivalent of "are these two robots co-located in the same zone".
- `robot(id).field` resolves `field` to a primitive published under `/robot/<id>/**`; type-checked against the field's declared type.
- `proximity(id) < const` is the protective-separation test.

### 3.3 Complexity & auditability

- Each `Predicate` is a fixed-depth boolean tree: leaf `Comparison` nodes, interior `and`/`or`/`not`. No recursion over data, no iteration. Evaluation is **O(1)** in the number of robots/zones.
- The total subscription set is the union of every primitive's key-expression — computable by a static walk of the ruleset. This is what makes the config PL d / SIL 2 auditable.

---

## 4. Mapping to `When` / `Trigger`

Replace `Trigger::pred: Option<String>` with a typed `on` reference plus a parsed `Predicate`:

```rust
// proposed shape (research only)
pub enum On {
    Site,                       // matches /<site>/* edge events
    Zone,                       // matches /<site>/<zone>/* edge events
    Robot { id: u32 },          // subscribes /robot/<id>/**
    HumanPresence,              // /<site>/<zone>/human_present
}

pub enum Predicate {
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
    Cmp { left: Operand, op: Op, right: Operand },
}

pub struct Trigger {
    pub on: On,                 // typed primitive ref (replaces free-text topic)
    #[serde(default)]
    pub pred: Option<Predicate>, // parsed tree (replaces Option<String>)
}
// When { all, any } is unchanged — it already composes AND/OR.
```

`When::all` = AND across triggers; `When::any` = OR. Each `Trigger` now carries a typed subscription (`on`) and an optional parsed `Predicate`.

### 4.1 Examples

**Protective stop when a human is present in the same zone and separation is too small:**

```toml
[[rules]]
name = "ssm_stop"
[when.all]
  on = "human_presence"
  pred = "human_presence == true and proximity(7) < 0.5"
[[rules.actions]]
topic = "stop/fleet/cmd"
qos = "reliable"
payload = { robot = 7 }
```

**Docking-speed cap (ISO 3691-4:2023, 0.3 m/s) only inside `site_a`:**

```toml
[[rules]]
name = "dock_speed_cap"
[when.all]
  on = "site"
  pred = 'site == "site_a" and robot(7).drive_state == "docking"'
[[rules.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { max_speed_m_s = 0.3 }
```

**Two robots must not share a zone:**

```toml
pred = "robot(7).zone same_zone_as robot(3).zone"
```

---

## 5. Safety-Standards Precedent

| Standard | Relevance to primitives |
|----------|------------------------|
| **ISO 3691-4:2023** (industrial trucks / driverless trucks) | Zones, Protected Areas, PDS; docking speed ≤ 0.3 m/s. `zone`/`site` + speed-cap action. https://www.iso.org/standard/82352.html |
| **ISO 10218-1** §5.10.4 / **ISO/TS 15066** §5.5.4 (SSM) | Safety Sensor Module: protective stop / speed-&-separation monitoring. `proximity(id) < const` = protective separation distance test. https://www.iso.org/standard/74364.html |
| **ISO 13855** | Positioning of safeguards w.r.t. approach speed; defines how the separation distance constant is derived. `proximity` floats feed this math upstream. https://www.iso.org/standard/77651.html |
| **ISO 13849-1 / IEC 61508** | PL d / SIL 2 — the auditability & determinism requirements this grammar is designed to satisfy (no loops/arithmetic/time). https://www.iso.org/standard/69864.html |
| **ANSI/RIA R15.08** | US industrial-mobile-robot safety; parallels ISO 3691-4 zone model. https://www.ansi.org/standards/robotics |
| **ROS 2 nav2 costmap** | De-facto zone/polygon costmap layering; `zone` mirrors costmap "keepout"/"safety" layers conceptually. https://docs.nav2.org/ |

### 5.1 Zenoh key-expressions = auditable subscription set

The fleet transport is Zenoh. Every primitive maps to an exact **key-expression**:

- `site` → `/<site>/*`
- `zone` → `/<site>/<zone>/*`
- `robot` → `/robot/<id>/**`
- `human_presence` → `/<site>/<zone>/human_present`
- `proximity` → `/robot/<id>/safety/separation`

Because key-expressions are first-class and enumerable, a static analysis pass over the ruleset can list **every topic the controller may subscribe to or publish on** — the core evidence for PL d / SIL 2 certification arguments.

---

## 6. Open Questions (for RFC)

1. How is `robot(id).field` type-checked against the stream schema? A published-field registry, or declared per-robot type?
2. Is `same_zone_as` sufficient, or do we also need `adjacent_zone(…)` for corridor coordination?
3. Should `Trigger::on` for `robot`/`proximity` be merged (proximity is just a field of robot)? Keeping them separate aids readability.
4. Predicate parser: hand-rolled recursive-descent vs. `pest`/PEG crate (new dependency — needs admin approval per AGENTS.md).

---

## References

- `src/rules.rs` — current `When`/`Trigger`/`Rule` definitions.
- ISO 3691-4:2023 — https://www.iso.org/standard/82352.html
- ISO 10218-1 — https://www.iso.org/standard/74364.html
- ISO/TS 15066 — https://www.iso.org/standard/67374.html
- ISO 13855 — https://www.iso.org/standard/77651.html
- ISO 13849-1 — https://www.iso.org/standard/69864.html
- IEC 61508 — https://www.iec.ch/
- ANSI/RIA R15.08 — https://www.ansi.org/standards/robotics
- ROS 2 nav2 — https://docs.nav2.org/
- Zenoh key-expressions — https://zenoh.io/
