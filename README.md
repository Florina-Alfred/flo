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
machine will auto-discover it via multicast scouting.

> **When multicast is blocked (Docker, WSL2, CI, VPN).** Multicast scouting
> (`224.0.0.224:7446`) is often filtered there and the client will hang at
> `registering with server...`. Use an explicit unicast endpoint:
>
> ```bash
> # Terminal 1: start server and note its Zenoh listen port (NOT the health port).
> # The Zenoh router listens on `tcp/127.0.0.1:0` → random port by default
> # (`src/auth.rs:152`, `src/transport.rs:86`). Find it via:
> #   ss -tlnp            # Linux: look for `127.0.0.1:<port>` owned by `flo-server` that is NOT the health port
> #   lsof -i -P -n | grep LISTEN   # macOS fallback
> # Or grep the log and use the port from the `health server listening` line to
> # exclude it, then pick the remaining `127.0.0.1` listener.
> # Once you have <zenoh-port>:
> FLO_HEALTH_ADDR=0.0.0.0:8080 cargo run --bin flo -- \
>   --robot-id robot-7 --config robot-7-config.toml --ruleset robot-7-rules.toml \
>   --auth-mode none --auth-allow-insecure \
>   --connect tcp/127.0.0.1:<zenoh-port>
> ```
>
> `--connect` is `src/cli.rs:54-56`; it sets `connect/endpoints` and forces
> Zenoh client mode (`tcp/127.0.0.1:<port>`). The value is the **Zenoh** port,
> not the health port (`FLO_HEALTH_ADDR`). `scripts/verify-readme-demo.sh`
> auto-discovers the Zenoh port this way and passes `--connect`; the README
> documents the same step explicitly.

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
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
```

And a ruleset — a TOML file of `[[rules]]` that declare sensor triggers and
actions (a trigger without `pred` is a pure topic match — it fires whenever that
topic arrives; add `pred` for payload predicates, see `docs/RULES.md` §8):

```toml
# robot-7-rules.toml — pure topic match (no pred); fires when both topics publish
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-7/local/bumper" },
  { topic = "robot-7/local/imu" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
# With predicate (typed, not string) — e.g. bumper pressed AND speed > 0.2:
# [[rules]]
# name = "e-stop-on-bumper"
# when.all = [
#   { topic = "robot-7/local/bumper", pred = { Comparison = { op = "Eq", lhs = { Field = "pressed" }, rhs = { Bool = true } } } },
#   { topic = "robot-7/local/imu",    pred = { Comparison = { op = "Gt", lhs = { Field = "speed_mps" }, rhs = { Float = 0.2 } } } },
# ]
# actions = [ { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } } ]
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
┌──────────────┐    Zenoh mesh (pub/sub + liveliness)
│  flo-server  │◄──────────────────────────────────────────┐
│  (fleet      │                                           │
│  coordinator)│  fleet/registration (put) ──────► register│
│              │  fleet/registration/response/* ◄── ack    │
│              │  fleet/deregistration (put) ───► deregister│
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
  { topic = "robot-7/local/human_present", pred = { Comparison = { op = "Lt", lhs = { Prim = "HumanPresence" }, rhs = { Float = 1.2 } } } },
]
actions = [
  { topic = "robot-7/local/drive", qos = "best_effort", payload = { speed_mps = 0.1 } },
]
```

**Predicate operators:** `Eq`, `Ne`, `Lt`, `Gt`, `Le`, `Ge` on typed operands
(`Bool`, `Int`, `Float`, `Str`, or `Prim` for sensor fields). Predicates are
typed under the hood (`Comparison`, `And`, `Or`, `Not` trees).

**Eval modes:** each trigger in `when.all` / `when.any` fires on **edge**
(state change) by default. Set `mode = "level"` to fire continuously while
true.

**Hot-reload:** rulesets are loaded at startup from `--ruleset <path>`. The
engine detects topic changes and rebuilds subscribers automatically (old
subscriptions are dropped, new ones created).

### Registration & state machine

Clients register with the server via Zenoh pub/sub: a `put` to
`fleet/registration` and a subscribed response on
`fleet/registration/response/{robot_id}` (deregistration uses
`fleet/deregistration` / `fleet/deregistration/response/{robot_id}`).
See `src/registration.rs` and `src/topic.rs`. The server tracks each client
through:

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
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

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

Every `flo` process (`flo` and `flo-server`) exposes an HTTP health server on
the address from `FLO_HEALTH_ADDR`:

- **Host default (no env):** `0.0.0.0:0` — OS-assigned random port on all
  interfaces (`src/runtime.rs:58`, `src/server.rs:60`). Each run logs
  `health server listening addr=0.0.0.0:<port>`.
- **Container default:** `0.0.0.0:8080` (`ENV FLO_HEALTH_ADDR=0.0.0.0:8080` in
  `Dockerfile`), so inside a container the health server is always on `8080`.

| Endpoint | Method | Meaning |
| --- | --- | --- |
| `/healthz` | GET | Liveness — `200 OK` while the process is up. |
| `/readyz`  | GET | Readiness — `200` once subsystems are started. |
| `/metrics` | GET | Prometheus exposition: `flo_uptime_seconds`, `flo_process_ready`, `flo_rule_eval_total`. |

### Probing on the host

On a bare host the port is random — grep the log for the bound address, then
curl it:

```bash
cargo run --bin flo -- --robot-id robot-7 --config robot-7-config.toml --auth-mode none --auth-allow-insecure 2>&1 | grep "health server listening"
# health server listening addr=0.0.0.0:54321
curl -f http://localhost:54321/healthz
curl -f http://localhost:54321/readyz
curl -f http://localhost:54321/metrics
```

To use a fixed port locally, set `FLO_HEALTH_ADDR` before starting:

```bash
FLO_HEALTH_ADDR=0.0.0.0:8080 cargo run --bin flo -- --robot-id robot-7 --config robot-7-config.toml --auth-mode none --auth-allow-insecure &
curl -f http://localhost:8080/healthz
```

In containers `8080` is already set, so `curl -f http://localhost:8080/healthz`
works inside the container (and with `-p 8080:8080` on the host).

### `flo --healthcheck` probe vs serve default mismatch

The one-shot probe `flo --healthcheck` (and `flo-server --healthcheck`, used as
Docker `HEALTHCHECK`) connects to `FLO_HEALTH_ADDR` with a **fallback default of
`127.0.0.1:8080`** (`src/bin/flo-client.rs:21`, `src/bin/flo-server.rs:14`).
This is intentionally different from the serve default:

- **Serve** (`src/runtime.rs:58`): `0.0.0.0:0` → random port if you don't set `FLO_HEALTH_ADDR`.
- **Probe**: `127.0.0.1:8080` → assumes you set `FLO_HEALTH_ADDR=0.0.0.0:8080`.

If you run on a host without setting `FLO_HEALTH_ADDR`, the probe hits
`127.0.0.1:8080` and gets `Connection refused` while the server is actually on a
random port. Fix: `export FLO_HEALTH_ADDR=0.0.0.0:8080` (same value for both
serve and probe) before starting, or grep the log and curl the random health
port as above. The server/client log `health server listening addr=...` is the
authoritative source for the host-mode address.

Structured JSON logging: `FLO_JSON_LOGS=1`. Verbosity: `RUST_LOG` (default
`info`).

## Building from source

```bash
cargo build                              # default features (no system deps)
cargo test --lib --tests                 # full test suite (count: cargo test --lib --tests -- --list | grep -c ': test')
cargo test --features media --lib --tests # media tests (requires GStreamer, see scripts/setup-dev.sh)
cargo clippy --all-targets -- -D warnings # lint (deny warnings)
cargo fmt --all -- --check               # format
```

The `media` feature (WebRTC video with GStreamer) is feature-gated — see
`scripts/setup-dev.sh` for system package install, then build with
`--features media` and test with `cargo test --features media --lib --tests`
(CI `media` job runs this plus `cargo test -- --ignored --list` to ensure the
ignored suite compiles). It builds on the `webrtc 0.21` line (Sans-I/O API), pinned
to `0.21.0-alpha.1` until a stable 0.21 publishes; the alpha status is tracked
upstream.

**Test validity (INFRA-09):** flaky sleeps are replaced by ready-gate
`oneshot`/`notify` where feasible (like `engine::subscribed`); where polling
remains, tests use deadline-based retry with bounded timeouts (e.g. 10s for
`core_loop` eval_counter, 2s for transport drop propagation, 20s for media
pipeline Playing) so CI load doesn't flap without slowing the suite.

## Safety posture

flo is the software pre-estop / coordination layer and is **not** safety-rated.
Missing or invalid config starts flo in a fail-safe state (empty ruleset, no
motion). Hardware STO / certified Safety-PLC remains the **primary** stop
authority. `#[forbid(unsafe_code)]` enforced on every source file.

Stale or missing sensor input **fails closed**: a missing payload field, a
`peer_id` mismatch, or an absent topic sample evaluates to `false` and triggers
no action (`src/engine.rs:72-74`, `src/engine.rs:131-135`). There is no
staleness timeout — `run_engine:279-288` ticks over `latest` forever — and no
assumed-hazard default. See `docs/RULES.md` §7 and `CONTEXT.md` for the full
posture.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
