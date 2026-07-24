# Server-Client Architecture Implementation Plan

> **For agentic workers:** Subagent-driven development. Each task is independent enough for a fresh subagent.

**Goal:** Migrate flo from a single monolithic binary to a two-binary crate (server + client) with config-driven pub/sub, registration, heartbeat monitoring, and config mutation pipeline.

**Architecture:** Single crate with two `[[bin]]` entries sharing `src/lib.rs`. Client binary loads config TOML + ruleset, registers with server via Zenoh Queryable, runs rule engine. Server binary loads server config TOML, accepts registrations, monitors liveliness for heartbeat, alerts on timeouts/poison.

**Tech Stack:** Rust, Zenoh (pub/sub + Queryable + liveliness), TOML config, serde, sha2.

## Global Constraints

- `#![forbid(unsafe_code)]` in every binary
- Edition 2024, MSRV 1.97.1
- No new dependencies without admin approval
- All IPC via Zenoh (no HTTP/gRPC for internal traffic)
- Config and ruleset are separate TOML files
- `simulate_sensors()` removed from library (moved to examples)

---

### Task 1: Crate structure + config structs

**Files:**
- Create: `src/bin/flo-client.rs`
- Create: `src/bin/flo-server.rs`
- Create: `src/bin/mod.rs`
- Remove: `src/main.rs`
- Modify: `Cargo.toml` (add second `[[bin]]`, change paths)
- Modify: `src/lib.rs` (add pub mods for cli, common, demo, device, health, production, server, mesh; remove `pub mod simulate`)
- Modify: `src/config.rs` (add `ClientConfig`, `ServerConfig` with serde + validation)
- Modify: `src/demo.rs` (remove `simulate::simulate_sensors` call)
- Modify: `src/production.rs` (remove `simulate` import, refactor for ClientConfig)
- Modify: `examples/mesh_demo.rs` (remove `simulate::simulate_sensors` call)
- Modify: `examples/custom_rules.rs` (remove `simulate::simulate_sensors` call)
- Remove: `src/simulate.rs` (optional — keep as internal helper if examples still reference, but remove from lib)

**Interfaces:**
- Consumes: Existing `RuleStore`, `Transport`, `cli::Args`, `engine`, `health`, `common`
- Produces: `ClientConfig` struct, `ServerConfig` struct, `src/bin/flo-client.rs` entry point, `src/bin/flo-server.rs` entry point

**Key decisions:**
- `ClientConfig` has TOML schema as designed in ticket #98: `[client]` (heartbeat_interval_ms required), `[server]` (endpoints optional), `[default_subscriptions]` (location x/y/z; zone site_id/zone_enter/zone_exit), `[default_publishers]` (location/zone with topic + period_ms)
- `ServerConfig` has `[[expected_clients]]` (robot_id only) per ticket #99
- `robot_id` from CLI flag, not config file
- Both binaries produce the same `Args` CLI parser

---

### Task 2: Registration protocol

**Files:**
- Create: `src/registration.rs` (client registration sender + server registration handler)
- Modify: `src/lib.rs` (add `pub mod registration`)
- Modify: `src/server.rs` (accept registrations via Queryable)
- Modify: `src/bin/flo-client.rs` (call registration after config load)
- Modify: `src/bin/flo-server.rs` (start registration handler)

**Interfaces:**
- Consumes: `ClientConfig`, `Transport`, `ServerConfig`
- Produces: `register_with_server()` in client, `run_registration_handler()` in server

**Key decisions (ticket #102):**
- Queryable on `fleet/registration` — request-reply
- Client payload: `{ robot_id, heartbeat_interval_ms, config (full config TOML), ruleset (full mutated ruleset) }`
- Server reply: simple `"ack"` or rejection
- Duplicate reject: same robot_id already registered → reject, log
- Server state: unknown / expected / registered / poisoned
- 3× retry with backoff on server unavailable
- Deregistration: client sends Query to `fleet/deregistration`

---

### Task 3: Liveliness heartbeat monitoring (server)

**Files:**
- Modify: `src/server.rs` (subscribe to liveliness `robot/*/client/liveliness`, track state)
- Modify: `src/registration.rs` (server-side client state management with poison logic)

**Interfaces:**
- Consumes: Transport, ServerConfig
- Produces: `run_heartbeat_monitor()` in server, `ClientState` enum

**Key decisions (ticket #103):**
- Liveliness-based detection (not polled pub/sub)
- Server subscribes to `robot/*/client/liveliness`
- Token declared: if registered → mark healthy
- Token undeclared: check grace period (5× state message intervals); if no deregistration → poison
- Poisoned → `fleet/alerts/heartbeat/{robot_id}` alert

---

### Task 4: Config mutation pipeline + hot-swap subscriber teardown

**Files:**
- Create: `src/mutation.rs` (SHA computation, ruleset mutation, subscriber lifecycle)
- Modify: `src/lib.rs` (add `pub mod mutation`)
- Modify: `src/config.rs` (hot-reload with teardown)
- Modify: `src/engine.rs` (support subscriber teardown and resubscribe)
- Modify: `src/bin/flo-client.rs` (call mutation pipeline on startup and hot-swap)

**Interfaces:**
- Consumes: Ruleset, RuleStore
- Produces: `MutatedRuleset` struct, `compute_sha()`, `startup_mutation()`, `handle_hot_swap()`

**Key decisions (ticket #104):**
- Only ruleset mutates (SHA + version injected); client config is never mutated
- SHA computed on raw (pre-mutation) ruleset for dedup comparison
- On hot-swap: tear down all old engine subscribers, restart with new topics
- Version-only changes are valid (SHA changes)
- Same raw SHA → reject as no-op

---

### Task 5: Unblocking tickets #102/#103/#104 via resolved decisions (cleanup)

No new files — this task integrates all pieces:
- Wire client startup: load config → load ruleset → compute SHA → mutate → register → start engine
- Wire server startup: load config → start registration handler → start liveliness monitor → start engine
- Wire hot-swap: new ruleset → SHA compare → tear down old subs → mutate → re-register with server → restart engine

N/A — folded into Tasks 2-4.
