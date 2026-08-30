# flo

**flo — Zenoh-mesh robot fleet in safe Rust — declarative rules, hot-reload, pub/sub + liveliness.**

Two binaries — `flo` (robot client runtime) and `flo-server` (fleet coordinator) — communicate over a [Zenoh](https://zenoh.io/) mesh. Each robot evaluates rules locally; the mesh carries registration, liveliness, and zone events. No central orchestrator, no K8s.

## Architecture

Why Zenoh mesh, not K8s: a robot fleet needs peer-to-peer pub/sub with liveliness tokens, not container scheduling. Zenoh gives decentralized discovery (multicast + explicit `--connect`), typed pub/sub, and per-robot liveliness (`robot/*/client/liveliness`) without a control plane round-trip. Rules keep firing from the last-good compiled set even on partition — K8s would be the wrong layer (orchestration vs coordination).

`flo` is the client runtime — `topic.rs` owns every topic builder/validator, `transport.rs` adapts the Zenoh session (router vs client mode via `--connect`), `semantic.rs` parses and validates a `RulesManifest` (`[site]`, `[zones]`, `[[rules]]`) into compiled `Rules`. The live set is an atomically-swapped `ActiveRules` (`Arc<Rules>` readers, `swap()` writers) with hot-reload. `flo-server` is `tokio::try_join!` supervision of `engine` + `hot-reload` + `registration` + `heartbeat monitor` + health server (`src/server.rs:68`, `src/runtime.rs:227`).

```
┌──────────────┐    Zenoh mesh (pub/sub + liveliness, not Queryable)
│  flo-server  │◄──────────────────────────────────────────┐
│  (fleet      │                                           │
│  coordinator)│  fleet/registration (put) ──────► register│
│  try_join!   │  fleet/registration/response/* ◄── ack    │
│              │  fleet/deregistration (put) ───► deregister│
│              │  robot/*/client/liveliness ──► heartbeat  │
│              │  fleet/alerts/heartbeat/* ──► poison      │
└──────────────┘                                           │
                                                            │
┌──────────────┐    ┌──────────────┐                       │
│  flo (robot-7)│    │  flo (robot-8)│                       │
│  runtime      │    │  runtime      │                       │
│  topic.rs     │    │  topic.rs     │                       │
│  transport.rs │    │  transport.rs │                       │
│  semantic.rs  │    │  semantic.rs  │                       │
│  → RulesManifest → Rules → ActiveRules → engine           │
│  subscribers  │    │  subscribers  │                       │
└──────────────┘    └──────────────┘                       │
         │                      │                           │
         │   robot-7/local/*    │   robot-8/local/*         │
         ▼                      ▼                           │
   [sensor data]          [sensor data]                     │
                                                            │
   Zone events are shared fleet-wide:                       │
   zone/*/entered, zone/*/cleared ─────────► zone tracker   │
```

## 30-second catch

The only thing a newcomer needs to trust the diagram — local verification, no server, no Docker, no K8s:

```bash
git clone https://github.com/Florina-Alfred/flo && cd flo
cargo test --lib --tests          # 170+ tests, 0 ignored
cargo run --bin flo -- --help     # client flags (no --video-* leak)
cargo run --bin flo -- rule check examples/rules/sample.toml  # OK: valid raw ruleset
# loopback demo without multicast/Docker — two terminals:
#   cargo run --bin flo-server -- --auth-mode none --auth-allow-insecure
#   cargo run --bin flo -- --config tests/fixtures/minimal-client-config.toml --connect tcp/127.0.0.1:<zenoh-port>
```

> **Multicast blocked on Docker/WSL/CI/VPN?** Zenoh scouting is on `224.0.0.224:7446` and is often filtered — the client hangs at `registering with server...`. Add `--connect tcp/127.0.0.1:<zenoh-port>` (the **Zenoh** port, not the health port). The `health server listening` line in the server log is *not* the Zenoh port — find the Zenoh listener via `ss -tlnp` (Linux) or `lsof -i -P -n | grep LISTEN` (macOS). See [Quickstart](#quickstart--5-minutes) and `scripts/verify-readme-demo.sh` (auto-discovers the Zenoh port with `ss`/`lsof` fallback).

`FLO_HEALTH_ADDR` (default `0.0.0.0:0` → random port on host, `0.0.0.0:8080` in containers via `Dockerfile`) is the health HTTP address, not the Zenoh mesh address — do not pass it to `--connect`.

## Quickstart — 5 minutes

Each ritual step → where it is documented → script entry point → macOS/Linux note. `scripts/verify-readme-demo.sh` runs this end-to-end with explicit `--connect` so it works with or without multicast.

### 1. Clone and verify locally (no server)

```bash
git clone https://github.com/Florina-Alfred/flo && cd flo
cargo test --lib --tests
cargo run --bin flo -- rule check examples/rules/sample.toml  # OK: valid raw ruleset
cargo run --bin flo -- rule check examples/rules/hrc-cell.toml
```

Docs: `docs/RULES.md` §6 (validate before deploy). Script: `verify-readme-demo.sh` steps 2–3.

### 2. Start the server (terminal 1)

Create a server config (who to expect; if omitted, server accepts all with a warning):

```toml
# server-config.toml
[[expected_clients]]
robot_id = "robot-7"

[[expected_clients]]
robot_id = "robot-8"
```

```bash
cargo run --bin flo-server -- \
  --config server-config.toml \
  --auth-mode none \
  --auth-allow-insecure
```

The server opens a Zenoh router (`tcp/127.0.0.1:0` → random port, `src/auth.rs:152`, `src/transport.rs:86`), starts the registration handler on `fleet/registration`, and monitors liveliness on `robot/*/client/liveliness`. Log: `flo-engine server mode started`. Script: `verify-readme-demo.sh` step 4 (background start + `health server listening` / `ss` port discovery).

### 3. Start a robot (terminal 2)

Copy the minimal client config and a ruleset:

```bash
cp tests/fixtures/minimal-client-config.toml robot-7-config.toml
cp examples/rules/sample.toml robot-7-rules.toml
cargo run --bin flo -- \
  --robot-id robot-7 \
  --config robot-7-config.toml \
  --ruleset robot-7-rules.toml \
  --auth-mode none \
  --auth-allow-insecure
# If multicast is blocked, append:
#   --connect tcp/127.0.0.1:<zenoh-port>
```

Multicast note: on Docker/WSL/CI/VPN add `--connect tcp/127.0.0.1:<zenoh-port>` (see 30-second catch). Find `<zenoh-port>` via `ss -tlnp` (Linux, shows `flo-server` owner) or `lsof -i -P -n | grep LISTEN` (macOS fallback) — exclude the health port (`health server listening addr=0.0.0.0:<port>`). `--connect` is `src/cli.rs:54`, sets `connect/endpoints` and forces Zenoh client mode. Script auto-discovers the Zenoh port this way.

### 4. Health probing (any terminal)

Every `flo` and `flo-server` exposes HTTP health on `FLO_HEALTH_ADDR`:

| Endpoint | Method | Meaning |
|----------|--------|---------|
| `/healthz` | GET | Liveness — `200 OK` while the process is up. |
| `/readyz`  | GET | Readiness — `200` once engine subscriptions are live. |
| `/metrics` | GET | Prometheus: `flo_uptime_seconds`, `flo_process_ready`, `flo_rule_eval_total`. |

```bash
# random port on host — grep the log for the bound address:
cargo run --bin flo -- --robot-id robot-7 --config robot-7-config.toml --auth-mode none --auth-allow-insecure 2>&1 | grep "health server listening"
# health server listening addr=0.0.0.0:54321
curl -f http://localhost:54321/healthz
# fixed port:
FLO_HEALTH_ADDR=0.0.0.0:8080 cargo run --bin flo -- --robot-id robot-7 --config robot-7-config.toml --auth-mode none --auth-allow-insecure &
curl -f http://localhost:8080/healthz
```

Probe vs serve default mismatch: serve defaults to `0.0.0.0:0` (random), the one-shot probe `flo --healthcheck` defaults to `127.0.0.1:8080` — set `FLO_HEALTH_ADDR` explicitly if you use the probe on a host.

## Rule authoring

`docs/RULES.md` is the deep dive. README shows the two shipped semantic rulesets and the raw fallback.

**Semantic (`RulesManifest`):** you write meanings (zones, humans, peers), `flo` compiles them to topics + predicates. Parsed by `semantic::parse_semantic` into `RulesManifest` (alias `SemanticDoc` kept one release, deprecated) and compiled via `semantic::compile` into `Rules` (live set is `ActiveRules`, alias `RuleStore` kept one release). See `CONTEXT.md` vocabulary.

HRC safety cell (`examples/rules/hrc-cell.toml`):

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
```

Warehouse fleet (`examples/rules/warehouse-fleet.toml`):

```toml
[site]
id = "dc-2"
frame = "dc-2/world"
[zones]
aisle-a = { shape = "rect", x = 0.0, y = 0.0, w = 1.2, h = 40.0 }
[[rules]]
name = "amr-yield-near-peer"
when.near = { entity = "8", dist = 2.0 }
actions = [ { slow_to = 0.3, qos = "best_effort" } ]
[[rules]]
name = "amr-slow-in-aisle"
when.in_zone = "aisle-a"
actions = [ { slow_to = 0.5, qos = "best_effort" } ]
```

Validate before deploy (semantic first, then raw fallback — same path `src/runtime.rs:310` uses at startup):

```bash
flo rule check examples/rules/hrc-cell.toml          # OK: valid semantic ruleset
flo rule check examples/rules/warehouse-fleet.toml    # OK: valid semantic ruleset
flo rule check examples/rules/sample.toml             # OK: valid raw ruleset
```

**Raw fallback (`examples/rules/sample.toml`):** when you need full control, write engine `[[rules]]` directly — topic + typed `pred` (`Field`, `Prim`, `Bool`/`Int`/`Float`/`Str`). A trigger without `pred` is a pure topic match (fires whenever that topic arrives); with `pred`, the typed predicate is evaluated and missing fields fail closed (see § Safety).

```toml
# pure topic match (fires when both topics publish):
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-7/local/bumper" },
  { topic = "robot-7/local/imu" },
]
actions = [ { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } } ]

# payload-aware (typed, not string):
# when.all = [
#   { topic = "robot-7/local/bumper", pred = { Comparison = { op = "Eq", lhs = { Field = "pressed" }, rhs = { Bool = true } } } },
#   { topic = "robot-7/local/imu",    pred = { Comparison = { op = "Gt", lhs = { Field = "speed_mps" }, rhs = { Float = 0.2 } } } },
# ]
```

## Configuration

Copy the minimal client config — every field is required (missing field is a fatal validation error, not a silent default):

```toml
# tests/fixtures/minimal-client-config.toml — copy this as your client.toml
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

Notes:

- **Zone verbs** are `entered`/`cleared` everywhere (engine subscriptions, client config defaults, validator `src/topic.rs:235` agree); the `enter`/`exit` spellings are rejected.
- **Topic form:** both `robot-7/local/bumper` (hyphen, 3-seg) and `robot/7/local/bumper` (slash, 4-seg) are accepted by `topic::check_topic_pattern`, but the slash form `robot/{id}/local/{resource}` is canonical per `CONTEXT.md`.
- **3-seg zone** `zone/cell-3/entered` and 5-seg `zone/site/cell/7/entered` are both valid; the fixture uses the 3-seg minimal form.
- **Robot ID** comes from `--robot-id` (or `FLO_ROBOT_ID` env), not from the config file — so the same template can be reused across the fleet.

Server config:

```toml
[[expected_clients]]
robot_id = "robot-7"

[[expected_clients]]
robot_id = "robot-8"
```

If omitted, the server accepts all clients with a warning.

## Contributing

Not on the first screen — see [`CONTRIBUTING.md`](CONTRIBUTING.md) for development setup, CI, and coverage. That file is the home for `CARGO_INCREMENTAL=0 -j2`, `cargo llvm-cov`, `actionlint`, `cargo test --features media`, and `cargo test -- --ignored --list` detail (low-disk, coverage, and ignored-suite notes live there, not in this README).

Quick sanity:

```bash
cargo test --lib --tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Install

After the catch — not before:

```bash
cargo install flo-rs   # installs both `flo` and `flo-server` binaries (crate is `flo-rs`, binary stays `flo`)
```

Verify the package:

```bash
cargo package           # after `cargo test --lib --tests` and `flo rule check`, before publish
```

Container images (multi-arch via `container.yml`, Cosign-signed, SPDX SBOM, SLSA provenance):

```
ghcr.io/<owner>/flo-server
ghcr.io/<owner>/flo-client
ghcr.io/<owner>/flo-server-media
ghcr.io/<owner>/flo-client-media
```

Built on `main` push and `v*` tags; PRs build but do not push. See `AGENTS.md` Container images and `.github/workflows/container.yml`.

## Safety posture

flo is the **software** pre-estop / coordination layer and is **not** safety-rated. Missing or invalid config starts flo in a fail-safe state (empty ruleset, no motion). Hardware STO / certified Safety-PLC remains the **primary** stop authority. `#![forbid(unsafe_code)]` enforced on every source file.

Stale or missing sensor input **fails closed**: a missing payload field, a `peer_id` mismatch, or an absent topic sample evaluates to `false` and triggers no action (`src/engine.rs:72-74`, `src/engine.rs:131-135`). There is no staleness timeout — `run_engine:279-288` ticks over `latest` forever — and no assumed-hazard default. See `docs/RULES.md` §7 and `CONTEXT.md` for the full posture.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
