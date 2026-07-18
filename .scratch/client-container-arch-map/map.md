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

- **04 (Observability):** Health via per-pod zenoh `LivelinessToken`
  (`robot/<id>/client/liveliness`, auto-cleared on process death). k8s probes = HTTP
  `httpGet` from a tiny **axum** (tokio+hyper) health server (`/healthz`, `/readyz`
  also verifies session+token) — avoids exec-probe fork overhead at DaemonSet density.
  Logging = `tracing` + `tracing-subscriber`. All ferrous/no-unsafe; crate gets
  `#![forbid(unsafe_code)]`. See `issues/04-observability-notes.md`.
- **01 (Device access):** Use generic **Device Plugin** (squat/generic-device-plugin or
  smarter-device-manager) as a DaemonSet advertising serial/USB/video/I2C/GPIO; mount
  into the non-privileged `flo` DaemonSet. Minimal `securityContext`: `privileged:false`,
  `allowPrivilegeEscalation:false`, `capabilities.drop:[ALL]`, `readOnlyRootFilesystem`,
  `runAsNonRoot`, `seccomp RuntimeDefault`. hostPath `/dev` is fallback for fixed nodes.
  Safe-Rust crates confirmed (`serialport`, `v4l`, `i2cdev`, `sysfs_gpio`) — no `unsafe`
  on our side. Device I/O stays local; only commands/telemetry cross zenoh. See
  `issues/01-device-findings.md`.
- **02 (Rule engine design):** Config = **TOML** (serde+toml, safe Rust). Rules are
  **composable** (`when.all` / `when.any` over key-expr matches + payload predicates →
  one or more `actions` with explicit QoS class). Hot-reload via zenoh topic
  `robot/<id>/local/rules` (reliable+durable); engine swaps atomically behind `Arc`,
  in-flight actuations complete, bad parses rejected. Engine `#![forbid(unsafe_code)]`.
  See `issues/02-rule-engine-design.md` for the concrete TOML schema example.
- **03 (DaemonSet skeleton):** `deploy/flo-client-daemonset.yaml` authored — ConfigMap
  (bootstrap `rules.toml`) + DaemonSet (one pod per node). Encodes 01 (device plugin
  `devic.es/*` limits, non-privileged `securityContext`: drop ALL, readOnlyRootFS,
  runAsNonRoot, seccomp RuntimeDefault) and 02 (rules ConfigMap + zenoh hot-reload
  topic). Health via HTTP `/healthz` + `/readyz` (map 04). Skeleton for review, not
  cluster-validated. See `issues/03-daemonset-skeleton.md`.

## Not yet specified

- **WebRTC signaling** for class-3 video remains a separate future map (see
  transport-protocol-map fog) — how peers discover/exchange SDP under k8s.
- Per-site hardware inventory (exact `devic.es/*` device names, node affinity)
  — operational detail filled at deploy time, not a design decision.

## Out of scope

<!-- work ruled beyond the destination; closed, never graduates -->

- Kubernetes platform shape (Cilium vs Istio mesh, cluster networking) — separate map.
- WebRTC signaling mechanism — separate future map.
- Browser/video UI implementation — separate future map.
- The transport/protocol decision itself — already locked in transport-protocol-map.
- These belong to other efforts, not this client-container-architecture lock.
