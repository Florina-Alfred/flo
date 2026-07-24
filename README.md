# flo

**flo** is a robot fleet orchestration system written in safe Rust. Two binaries
— `flo-server` (fleet coordinator) and `flo-client` (robot agent) — communicate
over a [Zenoh] mesh to provide declarative, hot-reloadable rule execution,
registration, and heartbeat monitoring for a fleet of robots.

[Zenoh]: https://zenoh.io/

## Quick start — three-terminal demo

### Terminal 1: Start the server

Create a server config that tells the fleet coordinator which robots to expect:

```toml
# server-config.toml
[[expected_clients]]
robot_id = "robot-7"

[[expected_clients]]
robot_id = "robot-8"
```

Launch the server (dev-mode, no mTLS):

```bash
cargo run --bin flo-server -- \
  --config server-config.toml \
  --auth-mode none \
  --auth-allow-insecure
```

The server opens a Zenoh router, starts the registration handler on
`fleet/registration`, and monitors client liveliness on
`robot/*/client/liveliness`. It logs reachable endpoints — clients on the same
machine will auto-discover it via multicast.

### Terminal 2: Start the first robot

Create a client config for `robot-7`:

```toml
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/7/enter"
zone_exit = "zone/cell-3/7/exit"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
```

And a ruleset — a TOML file of `[[rules]]` that declare sensor triggers and
actions:

```toml
# robot-7-rules.toml
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-7/local/bumper", pred = "pressed == true" },
  { topic = "robot-7/local/imu",    pred = "speed_mps > 0.2" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
```

Launch the client:

```bash
cargo run --bin flo -- \
  --robot-id robot-7 \
  --config robot-7-config.toml \
  --ruleset robot-7-rules.toml \
  --auth-mode none \
  --auth-allow-insecure
```

The client joins the Zenoh mesh, declares its liveliness token, sends its
config to the server via `fleet/registration`, and starts the rule engine.

### Terminal 3: Start a second robot

Create the same files for `robot-8` (different topic paths, ruleset, and
robot-id), then launch:

```bash
cargo run --bin flo -- \
  --robot-id robot-8 \
  --config robot-8-config.toml \
  --ruleset robot-8-rules.toml \
  --auth-mode none \
  --auth-allow-insecure
```

The server now tracks both clients. If a client's liveliness token drops
unexpectedly, the server transitions it to the **Poisoned** state and publishes
an alert on `fleet/alerts/heartbeat/{robot_id}`.

## Architecture

```
┌──────────────┐    Zenoh mesh (pub/sub + queryable + liveliness)
│  flo-server  │◄──────────────────────────────────────────┐
│  (fleet      │                                           │
│  coordinator)│  fleet/registration ──────► register      │
│              │  fleet/deregistration ───► deregister     │
│              │  robot/*/client/liveliness ──► heartbeat  │
│              │  fleet/alerts/heartbeat/* ──► poison      │
└──────────────┘                                           │
                                                           │
┌──────────────┐    ┌──────────────┐                       │
│  flo-client  │    │  flo-client  │                       │
│  (robot-7)   │    │  (robot-8)   │                       │
│              │    │              │                       │
│  rule engine │    │  rule engine │                       │
│  subscribers │    │  subscribers │                       │
└──────────────┘    └──────────────┘                       │
        │                      │                           │
        │   robot-7/local/*    │   robot-8/local/*         │
        ▼                      ▼                           │
  [sensor data]          [sensor data]                     │
                                                           │
  Zone events are shared fleet-wide:                       │
  zone/*/entered, zone/*/cleared ─────────► zone tracker   │
```

## Key concepts

### Rules

Rules are declarative TOML documents. Each rule has a name, a `when` condition,
and one or more `actions`:

```toml
[[rules]]
name = "slow-near-human"
when.all = [
  { topic = "robot-7/local/human_present", pred = "presence < 1.2" },
]
actions = [
  { topic = "robot-7/local/drive", qos = "best_effort", payload = { speed_mps = 0.1 } },
]
```

**Predicate operators:** `==`, `!=`, `>`, `>=`, `<`, `<=` on string, float, and
boolean operands. Predicates are typed under the hood (`Comparison`, `And`,
`Or`, `Not` trees).

**Eval modes:** each trigger in `when.all` / `when.any` fires on **edge**
(state change) by default. Set `mode = "level"` to fire continuously while
true.

**Hot-reload:** rulesets are loaded at startup from `--ruleset <path>`. The
engine detects topic changes and rebuilds subscribers automatically (old
subscriptions are dropped, new ones created).

### Registration & state machine

Clients register with the server via a Zenoh Queryable on `fleet/registration`.
The server tracks each client through:

```
Unknown  ──►  Expected  ──►  Registered  ──►  Poisoned
                  │                              │
                  └── (from server config)        └── (liveliness drop)
```

- **Expected:** robot_id listed in the server's `[[expected_clients]]`.
- **Registered:** client sent a valid registration payload and the server
  accepted it.
- **Poisoned:** client's liveliness token dropped without a clean
  deregistration. Subsequent registration attempts are rejected.

### Semantic rules (industrial)

For higher-level authoring — against zones, sites, robot proximity, and
human presence — use the semantic document format:

```toml
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
```

Validate before deploy:

```bash
flo rule check examples/rules/hrc-cell.toml
flo rule check examples/rules/warehouse-fleet.toml
```

See `docs/RULES.md` for the full semantic guide.

## Configuration

### Client config (`--config`)

Every field is required (missing fields are a fatal validation error):

```toml
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/7/enter"
zone_exit = "zone/cell-3/7/exit"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
```

Robot ID comes from `--robot-id` (or `FLO_ROBOT_ID` env), not from the config
file — so the same config template can be used across the fleet with only the
robot-id flag changing.

### Server config

```toml
[[expected_clients]]
robot_id = "robot-7"

[[expected_clients]]
robot_id = "robot-8"
```

If omitted, the server accepts all clients with a warning.

## Health & observability

Every `flo` client exposes an HTTP server on `0.0.0.0:8080`:

| Endpoint | Method | Meaning |
| --- | --- | --- |
| `/healthz` | GET | Liveness — `200 OK` while the process is up. |
| `/readyz`  | GET | Readiness — `200` once subsystems are started. |
| `/metrics` | GET | Prometheus exposition: `flo_uptime_seconds`, `flo_process_ready`, `flo_rule_eval_total`. |

```bash
curl -f http://localhost:8080/healthz
curl -f http://localhost:8080/readyz
curl -f http://localhost:8080/metrics
```

Structured JSON logging: `FLO_JSON_LOGS=1`. Verbosity: `RUST_LOG` (default
`info`).

## Building from source

```bash
cargo build          # default features (no system deps)
cargo test           # 56+ tests
cargo clippy         # lint (deny warnings)
cargo fmt            # format
```

The `media` feature (WebRTC video with GStreamer) is feature-gated — see
`scripts/setup-dev.sh` for system package install, then build with
`--features media`.

## Safety posture

flo is the software pre-estop / coordination layer. Missing or invalid config
starts flo in a fail-safe state. Hardware STO / certified Safety-PLC remains
the primary stop authority. `#[forbid(unsafe_code)]` enforced on every source
file.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
