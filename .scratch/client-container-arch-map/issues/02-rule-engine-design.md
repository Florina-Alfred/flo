# Ticket 02: Design the local rule engine (format, schema, hot-reload)

Label: `wayfinder:grilling`
Status: resolved
Blocked by:

## Question

Lock the design of the in-container local rule engine: the declarative config
format, the rule schema, and the hot-reload wiring.

Resolve via `/grilling` + `/domain-modeling` with the human. Decide:
- Config format: TOML vs YAML vs JSON (Destination says "declarative config";
  pick the concrete one and why).
- Rule schema: how a rule expresses trigger (zenoh topic/key-expr + condition on
  payload) → action (publish to actuator topic, possibly with QoS class). Should
  rules compose (AND/OR of triggers)? Should a rule reference `stop/**` vs
  `lidar/**` QoS classes explicitly?
- Hot-reload: which zenoh topic carries rule updates, how the engine applies them
  atomically without dropping in-flight actuations, and what happens to a running
  evaluation when its rule is removed.
- Keep it ferrous/no-unsafe: the engine is pure safe Rust; flag if any chosen
  config crate (serde + toml/yaml/json) forces unsafe (it won't).

This is a HITL ticket — resolves only through live exchange; the agent must not
answer its own questions. Outcomes feed ticket 03 (DaemonSet skeleton) and the
future implementation.

## Resolution (grilled with human)

Locked design:

- **Config format: TOML.** Ergonomic, strongly-typed via `serde` + `toml`; nests
  rule tables cleanly. Same safe-Rust stack as the rest (no `unsafe` on our side).
- **Rule schema: composable (AND/OR).** Each rule has a `when` block that is a
  boolean expression over trigger conditions (zenoh key-expr matches + optional
  payload predicates), composed with `all`/`any`. Firing evaluates to one or more
  `actions` (publish to an actuator key-expr, with an explicit QoS class so the
  rule decides reliable-or-best-effort). Key-exprs use the locked namespaces
  (`robot/<id>/local/**`, `stop/**` class 1, `lidar/**` class 2).

  Concrete TOML example:

  ```toml
  [[rules]]
  name = "e-stop-on-bumper"
  when.all = [
    { topic = "robot/7/local/bumper", pred = "pressed == true" },
    { topic = "robot/7/local/imu",     pred = "speed_mps > 0.2" },
  ]
  actions = [
    { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
  ]

  [[rules]]
  name = "lidar-block-slowdown"
  when.any = [
    { topic = "lidar/fleet/scan", pred = "min_range_m < 0.5" },
  ]
  actions = [
    { topic = "robot/7/local/drive", qos = "best_effort", payload = { speed_mps = 0.1 } },
  ]
  ```

- **Hot-reload: zenoh topic swap.** A `robot/<id>/local/rules` key-expr carries the
  full new TOML config (published reliable+durable). The engine subscribes, parses,
  and **atomically swaps** the active rule set behind an `Arc<Rules>` (readers hold
  the old `Arc` until their in-flight evaluation finishes — no actuation is dropped).
  A removed rule simply stops matching on the next evaluation; an in-flight action
  already published is unaffected. A bad TOML parse is rejected and the old rules
  stay active (logged via `tracing`).

- **Ferrous confirmation:** `serde` + `toml` + `serde_json`/value are safe Rust; the
  engine holds `#![forbid(unsafe_code)]`. No unsafe obligation on our side.

