# Ticket 02: Design the local rule engine (format, schema, hot-reload)

Label: `wayfinder:grilling`
Status: open
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
