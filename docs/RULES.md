# `flo` Rules — A Simple, In-Depth Guide

This guide explains how to write rules for `flo` from scratch. It is the companion to the
short overview in the [README](../README.md#semantic-rules-industrial). If you just want to
copy a working example, jump to [Two complete examples](#two-complete-examples).

---

## 1. The mental model in one paragraph

A **rule** is an `if → then` statement for your robot cell.

> **IF** some condition about the world is true (a human is close, the robot is in a
> restricted zone, another robot is nearby) **THEN** `flo` publishes a command (slow down,
> stop, resume).

You write rules in plain **TOML** against *meanings* — zones, humans, peers — not against raw
network addresses. `flo` compiles your meaning into the exact Zenoh topics it subscribes to and
publishes. The rule engine runs **locally on the robot**, so a stop command fires even if the
network or control plane is down.

That is the whole idea. The rest of this guide is the vocabulary.

---

## 2. The shape of a rules file

Every rules file has three optional parts and a list of rules:

```toml
[site]                       # who/where this robot is
id = "cell-7"                # site id — becomes part of every topic name
frame = "cell-7/world"       # coordinate frame name (documentation; 1 frame per site)

[zones]                      # named places in the plant
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }

[[rules]]                    # one rule
name = "..."                 # human-readable name (also the log label)
when.near_human = 1.2        # the condition
actions = [ { slow_to = 0.1, qos = "best_effort" } ]   # the command
```

- `[site].id` is **required** for compilation (it namespaces the generated topics).
- `[zones]` is a lookup table: a name → a rectangle. You reference the name in `when.in_zone`.
- Each `[[rules]]` is one `if → then`.

---

## 3. Conditions (`when`)

A rule fires when its `when` is satisfied. You can write one simple condition, or compose
several.

### 3.1 Simple conditions

| Key | Meaning | Compiles to (conceptually) |
|-----|---------|----------------------------|
| `in_zone = "safety"` | robot is inside the named zone | *robot's zone is "safety"* |
| `not_in_zone = "safety"` | robot is NOT in the named zone | *robot's zone is not "safety"* |
| `near_human = 1.2` | a human is within 1.2 m | *nearest-human distance < 1.2* |
| `not_near_human = 1.5` | no human within 1.5 m | *nearest-human distance ≥ 1.5* |
| `near = { entity = "8", dist = 2.0 }` | peer robot "8" is within 2.0 m | *nearest-peer distance < 2.0* |
| `role = "operator"` | the entity's role is "operator" | *role == "operator"* |

Distances are always in **meters** and must be **greater than 0** (the validator rejects `≤ 0`).

### 3.2 Composing conditions

Two keywords combine conditions:

- `when.all = [ ... ]` — fire only when **every** condition is true (logical AND).
- `when.any = [ ... ]` — fire when **any one** condition is true (logical OR).

Each item inside `all` / `any` is itself a full condition block — so you can nest them.

```toml
[[rules]]
name = "protective-stop"
when.any = [
  { in_zone = "safety" },       # human entered the safety zone
  { near_human = 0.3 },         # OR human is dangerously close
]
actions = [ { estop = true } ]
```

```toml
[[rules]]
name = "resume"
when.all = [
  { not_near_human = 1.5 },     # human cleared
  { not_in_zone = "safety" },   # AND robot left the safety zone
]
actions = [ { resume = true } ]
```

> **Rule:** a `when` with a flat key (`when.near_human = ...`) and a `when.all`/`when.any`
> block can be mixed in the same rule — the flat key is treated as an extra AND. For clarity,
> prefer one style per rule.

> **Nesting limit:** the runtime model is two-level — one AND group (`all`) optionally ANDed
> with one OR group (`any`). Nested blocks are flattened into that shape, and a shape the
> model cannot express (two OR groups ANDed together, or an AND-of-triggers as an OR
> element) is **rejected** at `flo rule check`/compile rather than silently mis-flattened.

---

## 4. Commands (`actions`)

A rule's `actions` is a list — fire as many commands as you need.

| Action | What it publishes | QoS default |
|--------|-------------------|-------------|
| `estop = true` | reliable **STOP** to `stop/fleet/cmd` | `reliable` (safe default) |
| `slow_to = 0.1` | slow to 0.1 m/s on `robot/{id}/local/drive` | `best_effort` |
| `resume = true` | resume motion on `robot/{id}/local/drive` | `reliable` |

You can override QoS explicitly: `actions = [ { estop = true, qos = "reliable" } ]`.
Use `reliable` for anything safety-critical (stop, resume); `best_effort` for smoothing
commands like slowdown.

An action with **no** known verb (`estop` / `slow_to` / `resume`) is rejected by validation.

---

## 5. What `flo` actually subscribes to (the topic contract)

You write meanings; `flo` generates exact Zenoh topic names. Knowing them helps when you
wire up sensors or read engine logs. For a robot with id `7`:

| Semantic condition | Topic `flo` watches | Predicate it checks |
|--------------------|--------------------|----------------------|
| `in_zone` / `not_in_zone` | `robot/{id}/local/zone` | `zone_id == "..."` |
| `near_human` / `not_near_human` | `robot/{id}/local/human_present` | `separation_distance < 1.2` |
| `near = { entity = "8", ... }` | `robot/{id}/local/proximity` | `peer_id == "8"` **and** `separation_distance < 2.0` |
| `role = "operator"` | `robot/{id}/local/role` | `role == "operator"` |

Someone (the robot's own fusion, or a sensor service) must **publish** those topics:

- `robot/{id}/local/zone` — the robot's current zone id (payload field `zone_id`).
- `robot/{id}/local/human_present` — nearest-human distance (payload field `separation_distance`).
- `robot/{id}/local/proximity` — nearest-peer id + distance. The payload must carry the peer
  robot id in a `peer_id` field; a `near = { entity = "8" }` condition matches only samples whose
  `peer_id` is `"8"`.
- `robot/{id}/local/role` — the entity's role (payload field `role`).

**Zone events** (for the `SameZoneAs` primitive): the engine also subscribes to
`zone/{zone_id}/entered` and `zone/{zone_id}/cleared` to learn which robots share a zone.
The event payload carries the robot id in a `robot_id` field. Zone-event verbs are
`entered`/`cleared` everywhere; the `enter`/`exit` spellings are rejected by the topic
validator.

This is why `flo` needs **no central server**: each robot publishes its own state and
liveliness; peers discover each other by topic.

The full topic contract — every topic the system publishes or subscribes to — lives in
`src/topic.rs` as constants and builder functions, all verified against the naming
validator.

---

## 6. Validate before you deploy

`flo rule check` parses and validates a rules file without running anything:

```bash
flo rule check examples/rules/hrc-cell.toml
# → OK: examples/rules/hrc-cell.toml is a valid semantic ruleset
flo rule check examples/rules/sample.toml
# → OK: examples/rules/sample.toml is a valid raw ruleset
```

It tries the semantic parser (`parse_semantic_auto` + `validate`) first; if that
fails it falls back to the raw engine parser (`Rules::from_toml` in
`src/cli.rs:215`, the same fallback `src/runtime.rs:310-330` uses at
startup). This lets one command validate both authoring layers:

- **Semantic files** (`hrc-cell.toml`, `warehouse-fleet.toml`): checked for
  negative/zero distances, unknown zones, missing verbs, empty `when`, malformed
  TOML.
- **Raw engine files** (`examples/rules/sample.toml`): checked that TOML parses
  as `[[rules]]` with `topic` + typed `pred` (`Field`/`Prim`/`Bool`/…),
  non-empty `when`, and topic names matching `src/topic.rs` (both
  `robot/{id}/local/...` and `robot-{id}/local/...` forms are accepted).

Exit code is `0` when valid, non-zero when not — wire it into your CI / GitOps step.

---

## 7. Safety behavior (fail-safe, by design)

`flo` is the **software** pre-estop / coordination layer — it is **not**
safety-rated. Hardware STO / a certified Safety-PLC remains the **primary**
stop authority; `flo` is the fast, non-safety-rated coordination layer in front
of it.

- **Missing or unreadable config** → `flo` starts in a fail-safe state (an empty ruleset, so it
  issues **no** motion commands) and logs `safe-state`. It does **not** crash and does **not**
  actuate unrestricted motion.
- **Invalid config** (fails `rule check`) → same fail-safe fallback; the last-good rules are
  kept.
- **Stale pose / lost human reading / missing field** → the engine **fails
  closed**: a missing payload field, a `peer_id` mismatch, or an absent topic
  sample evaluates to `false` and triggers **no action**
  (`src/engine.rs:72-74` fails closed on absent fields, `src/engine.rs:131-135`
  fails closed on peer mismatch). There is **no staleness timeout** —
  `run_engine:279-288` ticks over `latest` forever — and no assumed-hazard
  default (e.g. distance ≠ 0). A proximity rule with no fresh human distance
  does **not** assume the hazard is near; it simply does not fire.
- **Network / control-plane partition** → local rules keep running from the last-good compiled
  set. No cloud round-trip needed to keep acting.

Because stale/missing input yields no action, `flo` must not be the sole safety
stop.

---

## 8. Raw rules (no semantic layer)

If you prefer full control, `flo` also accepts plain runtime rules — topic names and typed predicates
directly. This is what the engine evaluates under the hood. Predicates are typed trees, not free-text
strings: payload fields are referenced with `Field("name")`, literals with `Bool`/`Int`/`Float`/`Str`,
and the five primitives with `Prim`. Both `robot/{id}/local/...` and
`robot-{id}/local/...` topic forms are accepted (`src/topic.rs`).

A trigger without `pred` is a pure topic match — it fires whenever that topic
arrives (see README quickstart). With `pred`, the engine evaluates the typed
predicate against the payload; missing fields fail closed (`false`, no action).

```toml
# Pure topic match (fires when both topics publish):
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-7/local/bumper" },
  { topic = "robot-7/local/imu" },
]
actions = [ { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } } ]

# With typed predicates (payload-aware):
# [[rules]]
# name = "e-stop-on-bumper"
# when.all = [
#   { topic = "robot-7/local/bumper",
#     pred = { Comparison = { op = "Eq", lhs = { Field = "pressed" },   rhs = { Bool = true } } } },
#   { topic = "robot-7/local/imu",
#     pred = { Comparison = { op = "Gt", lhs = { Field = "speed_mps" }, rhs = { Float = 0.2 } } } },
# ]
# actions = [ { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } } ]
```

The semantic layer is sugar on top of this. Mixed raw + semantic rules coexist in one ruleset —
and `flo rule check` validates either form (semantic first, then raw fallback).

---

## 9. Two complete examples

Both live in [`examples/rules/`](../examples/rules/) and pass `flo rule check`.

### 9.1 HRC safety cell (`hrc-cell.toml`)

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

Read it aloud: *slow when a human is within 1.2 m; protective-stop if a human enters the safety
zone or gets within 0.3 m; resume only after the human is 1.5 m away and the robot has left the
safety zone.*

### 9.2 Warehouse AMR fleet (`warehouse-fleet.toml`)

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

Read it aloud: *yield (slow to 0.3 m/s) when peer "8" is within 2 m; cap speed to 0.5 m/s in the
aisle; dock at 0.1 m/s at the station; protective-stop if peer "8" gets within 0.8 m.*

---

## 10. Quick reference

```toml
[site]
id = "..."                       # required; namespaces topics
frame = "..."                    # optional documentation string
[zones]
<name> = { shape = "rect", x, y, w, h }
[[rules]]
name = "..."                     # log label
when.<key> = <value>             # simple condition
when.all = [ { ... }, { ... } ]  # AND of conditions (nestable)
when.any = [ { ... }, { ... } ]  # OR of conditions (nestable)
actions = [ { estop = true, qos = "reliable" }
            { slow_to = 0.1, qos = "best_effort" }
            { resume = true, qos = "reliable" } ]
```

`when` keys: `in_zone`, `not_in_zone`, `near_human`, `not_near_human`, `near`, `role`.
Actions: `estop`, `slow_to`, `resume`.
