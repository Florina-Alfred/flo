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
Hardcoded validation patterns that topic strings in rules must match. Both
slash-separated (`robot/{id}/local/{sensor}`) and hyphenated
(`robot-{id}/local/{sensor}`) robot-id forms are accepted.

Accepted categories:

| Pattern | Example |
|---------|---------|
| `robot/{id}/local/{resource}` | `robot/7/local/bumper` |
| `robot/{id}/location/{axis}` | `robot/7/location/x` |
| `robot/{id}/zone` | `robot/7/zone` |
| `robot/{id}/site` | `robot/7/site` |
| `robot/{id}/client/liveliness` | `robot/7/client/liveliness` |
| `robot/{id}/signal/{peer}/{msgtype}` | `robot/self/signal/peer/offer` |
| `robot/{id}/local/cam{0,1,2}` | `robot/7/local/cam0` |
| `fleet/registration` | literal |
| `fleet/deregistration` | literal |
| `fleet/alerts/heartbeat/{id}` | `fleet/alerts/heartbeat/robot7` |
| `fleet/{site}/ruleset/{name}` | `fleet/cell-7/ruleset/acme` |
| `stop/{scope}/cmd` | `stop/fleet/cmd` |
| `lidar/{scope}/scan` | `lidar/fleet/scan` |
| `zone/{zone_id}/entered\|cleared` | `zone/cell-3/entered` |
| `zone/{site}/{cell}/{id}/{event}` | `zone/site/cell/7/enter` |

Implementation: `topic::check_topic_pattern()` in `src/topic.rs`.

### Error span
(line, column) pair pointing into the source file, attached to each error.
Validation errors also carry a field path (e.g. `rules[2].when.all[0].topic`).
