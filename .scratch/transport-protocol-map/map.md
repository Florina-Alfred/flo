# Map: Transport & QoS Protocol for Robot Orchestration Platform

Label: `wayfinder:map`

## Destination

Lock the inter-container transport and the 3 QoS class mappings for the robot
orchestration platform, before any k8s or client-architecture work begins.

Decided at chart time:
- **Zenoh** carries QoS class 1 (STOP: reliable, ordered, durable — never drop)
  and class 2 (lidar: best-effort, drop-allowed, lightweight).
- **WebRTC** carries class 3 (live camera video), chosen for future browser-UI support.
- QoS mapping is **locked as stated** above.
- **Hard constraint:** build with ferrous (safety Rust); no `unsafe` anywhere in our code.
- **Scope:** transport + QoS only. K8s platform, client-container arch, Cilium/Istio
  mesh, and the video UI are out of scope here (future maps).

## Notes

- Domain: robotics orchestration over Kubernetes.
- Skills every session should consult: `/grilling`, `/domain-modeling`, `/research`.
- Standing preferences: ferrous / no-unsafe is non-negotiable; prefer peer-driven
  actuation (container decides locally when a network sensor triggers an actuator).
- Issue tracker: local markdown (`.scratch/<feature>/`). Tickets are the files
  under `issues/`; the map body is the index. `Status:` line = open/claimed/resolved.
  `Blocked by:` line lists ticket numbers that must resolve first.

## Decisions so far

<!-- index — one line per resolved ticket, gist + link to the ticket holding detail -->

- **02 (WebRTC crate):** Adopt webrtc-rs (`webrtc`/`rtc`, pure safe Rust, no
  `unsafe` on our side, ferrous-compatible). Rejected livekit (unsafe libwebrtc
  FFI). Signaling deferred — see `issues/02-webrtc-findings.md`.
- **01 (Zenoh topology & crate):** Adopt `zenoh` 1.9.0 (pure safe Rust, ferrous-clean
  if `shared-memory` feature stays off; enable `unstable` only for `Reliability` enum).
  Topology **hybrid**: local P2P peer mesh for sensor→actuator (class-1 STOP stays
  on-node); `zenohd` routers at cluster/edge for durable cross-cluster STOP. QoS per
  message: class 1 = `Reliable`+`CongestionControl::Block`+high `Priority`+router
  Storage; class 2 = `BestEffort`+`Drop`+low `Priority`. **One session** multiplexes
  both via key-expr namespaces (`stop/**` vs `lidar/**`). See `issues/01-zenoh-findings.md`.
- **03 (Lock Zenoh topology):** Hybrid topology **locked** (matches 01 + user lean).
  Key-expr namespace prefixes **locked**: `robot/<id>/local/**` (on-node peer mesh),
  `fleet/**` (router-backed cross-cluster), with `stop/**` (class 1) and `lidar/**`
  (class 2) marking QoS class. See `issues/03-lock-zenoh-topology.md`.
- **04 (Ferrous build spike):** Spike builds clean (branch `research/transport-spike`,
  commit `92fe28a`). Our `src/` has **zero `unsafe`**; zenoh QoS builder API matches
  research (Reliable/Block/InteractiveHigh for stop, BestEffort/Drop/DataLow for lidar).
  `tokio` + `webrtc` + `zenoh`(unstable) integrate. **Caveat:** the `ferrous` safety
  compiler isn't installed in this env — final ferrous pass is a documented confirm
  step, not run. Crate-internal unsafe in deps (tokio/socket2/etc.) is acceptable per
  the hard constraint, which covers *our* code only. See `issues/04-ferrous-build-spike.md`.

## Not yet specified

- Zenoh **topology/deployment** shape (hybrid vs P2P peer mesh vs router backbone)
  for class 1 & 2 traffic — hangs on ticket 01 (zenoh deployment research).
- Whether a **single zenoh session** multiplexes both QoS classes or two sessions
  per class — hangs on 01.
- WebRTC **signaling** mechanism (how peers discover/exchange SDP) under k8s —
  out of scope for transport-lock but needed before class-3 code; note for future map.
- Concrete **Rust crate choice** for zenoh and its ferrous/unsafe posture
  — hangs on 01 (webrtc choice resolved in 02).

## Out of scope

<!-- work ruled beyond the destination; closed, never graduates -->

- Kubernetes platform shape (Cilium vs Istio mesh, DaemonSet vs Deployment).
- Client container architecture (how sensors/actuators are piped as volumes,
  the in-container decision loop).
- Browser/video UI implementation.
- These belong to separate future wayfinder maps, not this transport-lock effort.
