# Topic-Contract & Hybrid Transport Topology for `flo-engine`

**Ticket:** #67 — Zenoh topic/key-expression contract & p2p + cloud-router topology
**Status:** research / proposal
**Grounding:** `src/transport.rs`, `src/rules.rs` (`flo` crate)

---

## 1. Scope & how this grounds in `flo`

`src/transport.rs` already locks a key-expression namespace and QoS model:

- `robot/{id}/client/liveliness` — per-robot liveliness token (`transport.rs:11`)
- `robot/{id}/local/rules` — hot-reload ruleset key (`transport.rs:12`)
- `robot/{id}/signal/{peer}/{offer,answer,ice}` — WebRTC signaling, class-3 (`transport.rs:17-20`)
- QoS is **per-put**, not per-topic: `Qos::Reliable` → class 1 (Reliable+Block+InteractiveHigh), `Qos::BestEffort` → class 2 (BestEffort+Drop+DataLow) (`transport.rs:73-92`, `rules.rs:5-10`)

`src/rules.rs` defines `Trigger{ topic, pred }` and `Action{ topic, qos, payload }` — i.e. a
ruleset already references key-expressions directly, so **subscription scoping by ruleset is a
natural fit**: the engine declares one subscriber per distinct trigger topic a ruleset names.

This doc extends that locked namespace to cover robot state, site/zone events, and pins the
hybrid p2p/router topology.

---

## 2. Key-expression syntax (pinned)

Zenoh key-expressions (KEs) are forward-slash paths with two wildcards
([spec.zenoh.io](https://spec.zenoh.io/spec/1.0.0/concepts/key-expressions.html),
[zenoh.io key-expressions](https://corsaro.me/en/zenoh/book/core-concepts/key-expressions/)):

- `*` matches exactly **one** chunk (`a/*/c` ≠ `a/b/d/c`).
- `**` matches **zero or more** chunks (`a/**/c` includes `a/c`, `a/b/c`, `a/b/d/c`).
- No trailing `/`, no empty chunks, no adjacent wildcards.

`.intersects()` / `.includes()` on `KeyExpr` let a router decide at route-declaration time
whether a subscriber's KE overlaps a publisher's key — this is what makes subscription
scoping cheap and exact ([docs.rs/zenoh key_expr](https://docs.rs/zenoh/latest/zenoh/key_expr/index.html)).

**Casing:** snake_case segments, mirroring ROS 2 / DDS naming precedent
([design.ros2.org naming](https://design.ros2.org/articles/topic_and_service_names.html)) and
`flo`'s existing `robot/{id}/local/rules`. Robot ids are stable strings (e.g. UUID or serial).

---

## 3. Pinned namespace

```
robot/{robot_id}/local/{signal}        # robot self-state, class 1/2 (local mesh)
site/{site_id}/{event}                # site/zone event bus, class 1
zone/{zone_id}/{event}                # finer-grained zone bus, class 1
fleet/{cmd}                           # fleet-wide commands from engine, class 1
robot/{robot_id}/cmd                  # targeted command, class 1
robot/{robot_id}/signal/{peer}/{kind} # WebRTC signaling (class 3) — LOCKED, transport.rs
robot/{robot_id}/client/liveliness    # liveliness token — LOCKED, transport.rs
robot/{robot_id}/local/rules          # ruleset hot-reload — LOCKED, transport.rs
```

### 3.1 Robot state — `robot/{robot_id}/local/{signal}`

Per the ticket, robots publish state under `robot/<robot_id>/<signal>`. `local/` is kept as the
first segment (matching the locked `local/rules`) to separate self-produced state from
cross-robot addressing, and so a peer can subscribe `robot/{id}/local/**` for one robot or
`robot/**/local/proximity` fleet-wide.

| signal            | payload shape | QoS     | example |
|-------------------|---------------|---------|---------|
| `proximity`       | float (m)     | class 2 | `{"range_m": 1.23}` |
| `zone`            | string        | class 1 | `{"zone_id": "z12"}` |
| `pose`            | float×3 + q   | class 2 | `{"x":1.0,"y":2.0,"theta":0.5}` |
| `bumper`          | bool          | class 1 | `{"pressed": true}` |
| `human_present`   | bool          | class 1 | `{"present": true}` |
| `battery`         | float (%)     | class 2 | `{"pct": 78.4}` |
| `velocity`        | float         | class 2 | `{"mps": 0.4}` |

Key-expr examples: `robot/7/local/proximity`, `robot/7/local/bumper`, `robot/**/local/pose`.

### 3.2 Site & zone event bus — `site/{id}/{event}`, `zone/{id}/{event}`

Site/zone brokers (or the engine acting as broker) publish discrete events:

| key-expression                       | payload | QoS     | meaning |
|--------------------------------------|---------|---------|---------|
| `site/{site_id}/entered`             | string  | class 1 | `{"robot_id":"7"}` |
| `site/{site_id}/human_present`       | bool    | class 1 | `{"present":true}` |
| `zone/{zone_id}/entered`             | string  | class 1 | `{"robot_id":"7"}` |
| `zone/{zone_id}/human_present`       | bool    | class 1 | `{"present":true}` |
| `zone/{zone_id}/cleared`             | string  | class 1 | `{"robot_id":"7"}` |

Wildcards scope naturally:
- whole site: `site/3/**`
- all human-presence events anywhere: `**/human_present`
- one zone's life-cycle: `zone/z12/**`

### 3.3 Commands — `fleet/{cmd}`, `robot/{id}/cmd`

Actuator/stop actions from the engine. These are class 1 (Reliable+Block) per the locked
decision, matching `Action.qos = Reliable` in `rules.rs`. Example:
`robot/7/cmd` → `{"action":"stop"}`; `fleet/stop_all` → `{"reason":"human_present"}`.

---

## 4. Hybrid p2p + router topology (FAIL-SAFE requirement)

Zenoh supports three node roles composable into arbitrary topologies, including a **Hybrid**
of local peer meshes under a federated router backbone
([spec.zenoh.io topologies](https://spec.zenoh.io/spec/1.0.0/architecture/topologies.html),
[zenoh deployment](https://zenoh.io/docs/getting-started/deployment),
[industrial IoT ref arch 2026](https://iotdigitaltwinplm.com/zenoh-industrial-iot-reference-architecture-2026)).

```
                 ┌──────────────────────────────┐
                 │  cloud flo-engine (ZENOH ROUTER)
                 │  zenohd @ cloud, federated    │
                 └──────────────┬───────────────┘
                                │ TLS/QUIC or TCP (WAN uplink)
            ┌───────────────────┴───────────────────┐
            │            site / cell edge           │
            │  ┌─────────┐      ┌─────────┐         │
            │  │ robot 7 │◄────►│ robot 8 │  (PEER  │
            │  │ (peer)  │ \    │ (peer)  │   mesh, │
            │  └────┬────┘  \   └────┬────┘  multicast
            │       │        \       │        scouting)
            │    (p2p direct links preferred)
            └───────┼────────────────┼──────────────┘
                    │ connects to router as FAIL-SAFE
                    │ alternate path + partition bridge
```

- **Default path = peer mesh.** Robots run `mode: "peer"` (already `flo`'s `loopback_config`
  default, `transport.rs:53-59`). Multicast + gossip scouting auto-meshes same-LAN robots with
  **zero hops** — lowest latency, offloads the cloud.
- **Alternate path = cloud router.** Each robot *also* connects (as a peer or client) to the
  cloud `flo-engine` Zenoh router. The router never sits in the hot p2p path, but is reachable
  when p2p cannot deliver.
- **Failover semantics.** In classic peer-to-peer routing a router does **not** broker between
  two directly-connected peers (zenoh issue #372). The failover we want is therefore at the
  *path* level, not transparent bridging:
  1. Engine subscribes to `robot/{id}/local/**` on **both** the local peer link and the cloud
     router link.
  2. A rule's `Trigger.topic` is declared once per *session*; `flo` opens two sessions (or one
     session with both peer and router endpoints configured via `connect.endpoints` +
     `scouting.gossip`).
  3. **Prefer p2p:** local peer mesh delivers first/cheapest. If a robot is partitioned from
     the peer mesh (e.g. Wi-Fi roam, LAN drop), the cloud-router subscription still carries the
     sample, so the rule still fires — the router bridges the partition.
  4. Zenoh de-duplicates by key+source, so a sample arriving on both paths is delivered once to
     the callback. No double-trigger.
- **Why this satisfies the requirement:** robots stay peer-to-peer for low latency and to
  offload cloud; the cloud router is a *fail-safe alternate path* and a *partition bridge*, and
  p2p is preferred with automatic fallback to router.

> Note (1.9 Regions): for larger fleets, migrate non-clique peer groups to multiple peer
> subregions under the router rather than relying on router failover brokering
> ([dora #2721](https://github.com/dora-rs/dora/issues/2721)). For `flo`'s cell-scale mesh,
> the direct peer mesh + router-uplink pattern above is sufficient.

---

## 5. Scoping client subscriptions by ruleset

A `Rules` document (`rules.rs:56-61`) is a list of `Rule`s, each with `When{ all, any }` of
`Trigger{ topic, pred? }` and `actions: Vec<Action>`. Scoping falls out directly:

1. **Collect trigger keys.** Walk the loaded `Rules`, gather every distinct `Trigger.topic`
   (e.g. `robot/7/local/bumper`, `zone/**/human_present`). This is the **subscription set**.
2. **Declare one subscriber per key.** For each distinct topic, call `Transport::subscribe`
   (`transport.rs:108`) once. Wildcards in a trigger (`zone/**/human_present`) collapse many
   concrete keys into a single cheap subscription — `.intersects()` is what the router uses to
   fan out, so a broad trigger costs one route entry, not N.
3. **Predicate filtering is local.** `Trigger.pred` (e.g. `pressed == true`) is evaluated in the
   engine callback, not by Zenoh — Zenoh only routes by KE, the engine filters by payload.
4. **Hot-reload = diff the set.** On ruleset update (`robot/{id}/local/rules`, already locked),
   undeclare subscribers whose keys vanished and declare new ones. Keeps the mesh's routing
   table minimal: a robot only pulls the keys its active ruleset references.
5. **Actions publish to their own keys** (`Action.topic` + `Action.qos`), independent of the
   subscription set — e.g. a rule triggered by `zone/z12/human_present` publishes
   `robot/7/cmd` with `qos: Reliable`.

This gives least-privilege data flow: no robot subscribes to keys it has no rule for.

---

## 6. Payload encoding

`flo` already serializes payloads as JSON bytes (`transport.rs:79`). Keep `AppJson`/`TextJson`
encoding; primitives are plain JSON scalars (bool/float/int/string) as shown in §3. Zenoh
auto-stamps an HLC timestamp on the first router hop
([zenoh abstractions](https://zenoh.io/docs/manual/abstractions)), giving total ordering for
free when samples traverse the cloud router.

---

## 7. References

- Zenoh key-expression spec — https://spec.zenoh.io/spec/1.0.0/concepts/key-expressions.html
- Zenoh Rust `key_expr` module — https://docs.rs/zenoh/latest/zenoh/key_expr/index.html
- Zenoh topologies (Hybrid) — https://spec.zenoh.io/spec/1.0.0/architecture/topologies.html
- Zenoh deployment / modes — https://zenoh.io/docs/getting-started/deployment
- Zenoh Book, Key Expressions — https://corsaro.me/en/zenoh/book/core-concepts/key-expressions/
- Zenoh Book, Peer/Client/Router modes — https://corsaro.me/en/zenoh/book/routing/
- Industrial IoT reference architecture (hybrid peer+router) — https://iotdigitaltwinplm.com/zenoh-industrial-iot-reference-architecture-2026
- Peer failover brokering caveat — https://github.com/eclipse-zenoh/zenoh/issues/372
- ROS 2 / DDS topic naming precedent — https://design.ros2.org/articles/topic_and_service_names.html
- Zenoh abstractions (timestamps, encodings) — https://zenoh.io/docs/manual/abstractions
