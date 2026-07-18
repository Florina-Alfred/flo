# Map: Client-Container Architecture for Robot Orchestration Platform

Label: `wayfinder:map`

## Destination

Lock the client-container architecture for the robot orchestration platform —
how the per-robot software is packaged, deployed, and makes local actuation
decisions — before implementation starts.

Decided at chart time (pinned via grilling):
- **DaemonSet** — one pod per robot node, co-located with that node's hardware.
- **Device mounts** — sensors/actuators reach the container via `/dev` nodes,
  `hostPath`, or a k8s Device Plugin (the "as volumes" phrasing = loose; real
  mechanism is device exposure, not storage volumes).
- **In-container local rule engine** — a zenoh subscriber evaluates declarative
  rules mapping sensor input → actuator output and publishes actuator commands
  locally, with no external call needed for node-local reactions (satisfies the
  peer-driven-actuation requirement from the transport map).
- **Rules = declarative config** (TOML/YAML/JSON), loaded at start and
  hot-reloaded via a zenoh topic.
- **Observability** = k8s liveness/readiness probes + zenoh admin-space telemetry
  + structured logs.
- Reuses the locked transport map: zenoh hybrid topology, key-expr namespaces
  `robot/<id>/local/**`, `fleet/**`, `stop/**` (class 1), `lidar/**` (class 2).

## Notes

- Domain: robotics orchestration over Kubernetes.
- Skills every session should consult: `/grilling`, `/domain-modeling`, `/research`.
- Standing preferences: ferrous / no-unsafe is non-negotiable (carried from the
  transport map). Prefer peer-driven actuation; declarative, hot-reloadable rules.
- Depends on: transport-protocol-map (zenoh crate, QoS classes, topology, namespaces).
- Issue tracker: local markdown. Tickets under `issues/`; `Status:` = open/claimed/
  resolved; `Blocked by:` lists ticket numbers that must resolve first.

## Decisions so far

<!-- index — one line per resolved ticket, gist + link to the ticket holding detail -->

## Not yet specified

- Concrete **device-mount mechanism** per device class: which sensors/actuators
  use `/dev` node mounts, which need a Device Plugin, which need hostPath — hangs
  on ticket 01 (device-access research) and the actual hardware inventory.
- **Rule engine representation** — which config format (TOML vs YAML vs JSON) and
  what the rule schema looks like (trigger topic + condition + action topic) —
  hangs on ticket 02 (rule engine design).
- **Hot-reload wiring** — which zenoh topic carries rule updates and how the engine
  applies them without dropping in-flight actuations — hangs on 02.
- **DaemonSet pod spec shape** — resource requests, privileged/non-privileged
  posture, securityContext for device access, probe endpoints — hangs on 01 and 03.
- **WebRTC signaling** for class-3 video remains a separate future map (see
  transport-protocol-map fog).

## Out of scope

<!-- work ruled beyond the destination; closed, never graduates -->

- Kubernetes platform shape (Cilium vs Istio mesh, cluster networking) — separate map.
- WebRTC signaling mechanism — separate future map.
- Browser/video UI implementation — separate future map.
- The transport/protocol decision itself — already locked in transport-protocol-map.
- These belong to other efforts, not this client-container-architecture lock.
