# Context

## Domain terms

### rule check
Validates a ruleset file (semantic or compiled TOML) for correctness without
connecting to a Zenoh mesh or running the engine. Produces human-readable output
by default; `--json` flag produces structured JSON. Exit code 0 on valid, 1 on
invalid.

### rule compile
Compiles a semantic ruleset document into the engine's `[[rules]]` format.
Outputs the compiled TOML ruleset to stdout by default; `--verbose` additionally
prints the parsed AST.

### Topic naming convention
Single source of truth lives in `src/topic.rs`: constants for fixed topics,
builder functions for parameterized ones (`topic::robot_local`, `topic::rules_key`,
`topic::signal_offer_key`, `topic::zone_entered`, …), and pattern constants for
subscriptions. Every topic the system publishes or subscribes to is built there —
no other module constructs topic strings — and this module's tests verify every
builder output against `topic::check_topic_pattern()`. Both slash-separated
(`robot/{id}/local/{sensor}`) and hyphenated (`robot-{id}/local/{sensor}`)
robot-id forms are accepted.

Accepted categories:

| Pattern | Example |
|---------|---------|
| `robot/{id}/local/{resource}` | `robot/7/local/bumper` |
| `robot/{id}/location/{axis}` | `robot/7/location/x` |
| `robot/{id}/zone` | `robot/7/zone` |
| `robot/{id}/site` | `robot/7/site` |
| `robot/{id}/client/liveliness` | `robot/7/client/liveliness` |
| `robot/{id}/signal/presence` | `robot/7/signal/presence` |
| `robot/{id}/signal/{peer}/{msgtype}` | `robot/self/signal/peer/offer` |
| `robot/{id}/local/cam{0,1,2}` | `robot/7/local/cam0` |
| `fleet/registration` | literal |
| `fleet/deregistration` | literal |
| `fleet/registration/response/{id}` | `fleet/registration/response/robot-7` |
| `fleet/deregistration/response/{id}` | `fleet/deregistration/response/robot-7` |
| `fleet/alerts/heartbeat/{id}` | `fleet/alerts/heartbeat/robot7` |
| `fleet/{site}/ruleset/{name}` | `fleet/cell-7/ruleset/acme` |
| `stop/{scope}/cmd` | `stop/fleet/cmd` |
| `lidar/{scope}/scan` | `lidar/fleet/scan` |
| `zone/{zone_id}/entered\|cleared` | `zone/cell-3/entered` |
| `zone/{site}/{cell}/{id}/entered\|cleared` | `zone/site/cell/7/entered` |

Zone-event verbs are `entered`/`cleared` everywhere (engine subscriptions, client
config defaults, and the validator agree); `enter`/`exit` are rejected.

Implementation: `topic::check_topic_pattern()` and the topic builders in `src/topic.rs`.

### Safety posture (fail-closed, not safety-rated)
`flo` is the software pre-estop / coordination layer and is **not** safety-rated.
Hardware STO / Safety-PLC is the primary stop authority.

- Missing/unreadable/invalid config → fail-safe empty ruleset, no motion, log
  `safe-state`.
- Missing/stale sensor input → engine fails closed: `eval_tree` returns `false`
  on absent field (`src/engine.rs:72-74`) and `resolve_operand` returns `None`
  on `peer_id` mismatch (`src/engine.rs:131-135`); no action is published.
  No staleness timeout — `run_engine:279-288` ticks over `latest` forever —
  and no assumed-hazard default. A stale pose does **not** assume hazard near.
- Network partition → local rules keep running from last-good compiled set.

### Error span
(line, column) pair pointing into the source file, attached to each error.
Validation errors also carry a field path (e.g. `rules[2].when.all[0].topic`).
