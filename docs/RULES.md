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
wire up sensors or read engine logs. For a robot with id `7` at site `cell-7`:

| Semantic condition | Topic `flo` watches | Predicate it checks |
|--------------------|--------------------|----------------------|
| `in_zone` / `not_in_zone` / `role` | `fleet/cell-7/7/state` | `zone_id == "..."` / `role == "..."` |
| `near_human` / `not_near_human` | `fleet/cell-7/proximity/7/human` | `separation_distance < 1.2` |
| `near = { entity = "8", ... }` | `fleet/cell-7/7/nearest_peer` | `separation_distance < 2.0` |

Someone (the robot's own fusion, or a sensor service) must **publish** those topics:

- `fleet/{site}/{id}/state` — the robot's own pose/zone/role/speed.
- `fleet/{site}/proximity/{id}/human` — nearest-human distance.
- `fleet/{site}/{id}/nearest_peer` — nearest-peer id + distance.

This is why `flo` needs **no central server**: each robot publishes its own state and
liveliness; peers discover each other by topic.

---

## 6. Validate before you deploy

`flo rule check` parses and validates a rules file without running anything:

```bash
flo rule check examples/rules/hrc-cell.toml
# → OK: examples/rules/hrc-cell.toml is a valid semantic ruleset
```

It catches:
- negative or zero distances,
- an action with no known verb,
- a reference to a zone that isn't defined in `[zones]`,
- malformed TOML.

Exit code is `0` when valid, non-zero when not — wire it into your CI / GitOps step.

---

## 7. Safety behavior (fail-safe, by design)

`flo` is the **software** pre-estop / coordination layer. It is honest about its limits:

- **Missing or unreadable config** → `flo` starts in a fail-safe state (an empty ruleset, so it
  issues **no** motion commands) and logs `safe-state`. It does **not** crash and does **not**
  actuate unrestricted motion.
- **Invalid config** (fails `rule check`) → same fail-safe fallback; the last-good rules are
  kept.
- **Stale pose / lost human reading** → a proximity rule fails *safe* (assumes the hazard is
  near) rather than failing open.
- **Network / control-plane partition** → local rules keep running from the last-good compiled
  set. No cloud round-trip needed to keep acting.

Hardware STO / a certified Safety-PLC remains the **primary** stop authority. `flo` is the fast
software layer in front of it.

---

## 8. Raw rules (no semantic layer)

If you prefer full control, `flo` also accepts plain runtime rules — topic names and predicates
directly. This is what the engine evaluates under the hood:

```toml
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot/7/local/bumper", pred = "pressed == true" },
  { topic = "robot/7/local/imu",    pred = "speed_mps > 0.2" },
]
actions = [ { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } } ]
```

The semantic layer is sugar on top of this. Mixed raw + semantic rules coexist in one ruleset.

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
