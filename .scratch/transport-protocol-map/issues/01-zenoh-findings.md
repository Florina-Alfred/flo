# Ticket 01 Findings — Zenoh deployment topology & ferrous-compatible crate

Research branch: `research/zenoh-transport`
Resolution commit: see `git log` on this branch (single commit, this file).
Primary sources verified 2026-07-18 against:
- `docs.rs/zenoh/1.9.0` (crate docs, qos module, Publisher/Config/WhatAmI)
- `zenoh.io/docs/manual/abstractions/` and `zenoh.io/docs/manual/plugins/`
- `github.com/eclipse-zenoh/zenoh` README (main)
- `zenoh.io` blog posts (1.1.0 QoS overwrite, 1.8.x Kiyohime)
- crates.io `zenoh` dependency manifest

Zenoh latest stable at research time: **1.9.0** (docs.rs), dual-licensed EPL-2.0 / Apache-2.0.

---

## 1. Deployment topologies for a k8s robot cluster

Zenoh has a `mode` config (`WhatAmI`: `router` | `peer` | `client`) and lets nodes
form an arbitrary graph (mesh, star, clique). Three applicable patterns:

### A. P2P peer mesh
Every container runs in `peer` mode; peers discover and route directly with no
central node. Zenoh scouting auto-discovers peers (multicast/known endpoints).
- Pros: no single point of failure; lowest latency (sensor→actuator stays on-node
  or on-link); survives router loss; trivial horizontal scale.
- Cons: routing tables grow O(n²)-ish in a full mesh; no built-in durable storage
  or cross-cluster bridging without a router. Best when nodes are co-located
  (same pod/node/rack).

### B. Router backbone
One or more `zenohd` routers form the backbone; containers are `client` (or
`peer`) connecting to routers. Routers load plugins (storage, REST, ACL).
- Pros: centralized policy/ACL, durable **Storages**, clean cross-cluster/edge
  bridging, observability via admin space.
- Cons: routers are a (mitigable, via router mesh) bottleneck and SPoF; every
  local sensor→actuator hop may traverse a router, adding latency and a failure
  dependency that hurts *peer-driven actuation*.

### C. Hybrid (RECOMMENDED)
Local P2P on-node / same-rack among peers for time-critical traffic; routers at
cluster and edge boundaries for cross-cluster forwarding, durability, and
policy. `gateway.south = "auto"` puts peers/clients south of routers by default.
- Pros: gets both — local sensor→actuator loop never leaves the peer mesh
  (meets "decide locally" + class-1 never-drop stays local), while STOP-class
  commands that must fan out across the fleet or survive restarts go via routers
  (durable Storage + liveliness). Failure of a router does not break local
  actuation.
- Cons: two-tier mental model; need key-expr namespace discipline so local vs
  backbone traffic is separable (e.g. `robot/<id>/local/**` vs `fleet/**`).

**Lean: Hybrid.** For a container that decides locally when a network sensor
triggers an actuator, keep that path on the peer mesh (class 1 reliable, ordered,
local) and use routers only for cross-cluster/durable needs. This directly
satisfies the "peer-driven actuation" requirement and the never-drop class-1 goal.

---

## 2. Official `zenoh` Rust crate — ferrous / safety posture

- **Pure Rust, reference implementation.** The crate is the canonical Zenoh impl;
  other-language libs bind to it (except pure-C zenoh-pico). Compiles on Rust
  stable >= 1.75 (CI confirms stable toolchain).
- **Our `unsafe` exposure: NONE required.** The public API we use (open session,
  declare_publisher/subscriber, put, recv_async, QoS builder methods) is a normal
  safe async API. No `unsafe` block is needed on our side for pub/sub, query/reply,
  key expressions, or QoS configuration. (Shared-memory transport `zenoh-shm` is
  the one feature that touches `unsafe`-backed zero-copy; we simply do NOT enable
  the `shared-memory` feature. Default features do not include it.)
- **Ferrous posture:** The crate is written in safe-by-default Rust on stable.
  There is no `unsafe` obligation for our usage path. ferrous (the safety
  compiler) should pass our source as long as we (a) avoid the `shared-memory`
  feature, (b) keep our own code `unsafe`-free, and (c) accept that zenoh's
  transitive deps (tokio, socket2, etc.) are normal safe-Rust crates. We cannot
  guarantee zero `unsafe` in third-party deps, but our *code* need not contain any.
- **Caveat — `Reliability`:** The `Reliability` enum (`Reliable`/`BestEffort`) is
  currently gated behind the crate's `unstable` feature flag (confirmed in
  docs.rs 1.9.0: "Available on crate feature `unstable` only"). `unstable` is an
  *API-stability* marker, NOT `unsafe` — enabling it does not introduce `unsafe`
  on our side. But it means relying on `Reliability::Reliable` requires opting
  into `features = ["unstable"]`, accepting that the API may change.
  `CongestionControl` and `Priority` are NOT gated (stable).

### QoS knobs exposed (builder methods on Publisher / per-`put`)
From `zenoh::qos` and the Publisher builder:

| Knob | Enum / type | Maps to |
|------|-------------|---------|
| Reliability | `Reliability::{BestEffort, Reliable}` (feature `unstable`) | delivery guarantee |
| CongestionControl | `CongestionControl::{Drop, Block, BlockFirst}` | drop-allowed vs block-on-full-queue |
| Priority | `Priority::{RealTime, InteractiveHigh, InteractiveLow, DataHigh, Data, DataLow, Background}` (default `Data`) | scheduling queue |
| Ordering | inherent — Zenoh preserves per-key-expression ordering for reliable flows | ordered delivery |
| Express | bool (config/overwrite) | bypass some buffering for low-latency |
| Durability | NOT a per-message flag. Modeled via **Storages** (router plugin) + query/get; reliable pub/sub + router Storage gives durable, replayable state. | durability/history |

### Mapping to our two classes
- **Class 1 (STOP: reliable, ordered, durable, never-drop):**
  - `Reliability::Reliable`
  - `CongestionControl::Block` (never drop under congestion; default is `Drop`)
  - `Priority::InteractiveHigh` (or `RealTime` for the most critical)
  - Ordering is inherent for reliable flows.
  - Durability: route through a router with a **Storage** (or keep last-value via
    queryable) so a late/rebooted subscriber can recover the last STOP state.
  - Use `put` with these QoS, or set them on the `declare_publisher` builder.
- **Class 2 (lidar: best-effort, drop-allowed, lightweight):**
  - `Reliability::BestEffort`
  - `CongestionControl::Drop` (default)
  - `Priority::DataLow` / `Background`
  - No Storage/durability needed; freshness > completeness, so drops are fine.

Note: per-key-expression QoS **overwrite** is possible via the router config
`qos.publications` section (Zenoh 1.1.0+), useful to enforce class policy
centralized without touching app code — but our primary path is the builder API.

---

## 3. Single session vs two sessions for the two QoS classes

**A single `Session` can multiplex both classes.** QoS in Zenoh is per
*publisher* / per *put* / per key-expression, not per session. One session can
declare multiple publishers with different QoS, and subscribers receive with
whatever QoS the publisher set. Zenoh keeps one transmission queue per
`Priority` when QoS is enabled in config, so class-1 and class-2 traffic are
already isolated at the queue level within one session.

**Recommendation: one session, differentiated by publisher/put QoS + key-expr
namespace.** Use key-expr prefixes to make the class explicit and enforceable
(e.g. `stop/**` vs `lidar/**`). Two sessions buy nothing for QoS isolation (that
is per-message) and only add a connection, a runtime, and config complexity.
Reserve a second session only if you need a *different transport/endpoint* or
network partition (e.g. class-1 on an isolated real-time NIC) — then it's a
topology decision, not a QoS one.

---

## Key recommendation (gist)
- **Topology:** Hybrid — local peer mesh for sensor→actuator actuation (class 1
  stays on-node), routers at cluster/edge boundaries for durable cross-cluster
  STOP distribution.
- **Crate:** `zenoh` 1.9.0, pure safe Rust on stable. Our code needs **no
  `unsafe`**; ferrous-clean as long as we skip the `shared-memory` feature.
  Enable `unstable` feature only if we need the `Reliability` enum.
- **QoS classes:** express class 1 = `Reliable` + `CongestionControl::Block` +
  high `Priority` + router Storage for durability; class 2 = `BestEffort` +
  `Drop` + low `Priority`.
- **Sessions:** one `Session` per process; multiplex both classes via per-publisher
  QoS and key-expr namespace. Two sessions only if separate transports are needed.
