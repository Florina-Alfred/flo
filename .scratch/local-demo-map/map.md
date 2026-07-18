# Map: Foolproof Local Demo & Onboarding

Label: `wayfinder:map`

## Destination

Make the `flo` repo dead-simple to start and explore: a user with only Rust +
cargo can, with **one command**, watch the real rule engine react to a simulated
sensor over a real zenoh mesh on loopback — no cluster, no devices, no config.

Decided at chart time (pinned via grilling):
- **`cargo run` with no args = the demo.** Simulate mode + rule engine + embedded
  sample rules, by default. k8s/production mode requires explicit flags
  (`--robot-id`, `--config`, ...).
- **zenoh peer mode + loopback endpoints** for zero-config discovery: multiple
  `cargo run` on one host auto-scout each other over loopback. Matches the hybrid
  "local peer mesh" decision from the transport map (no router needed locally).
- **Embed the map-02 rules** (e-stop-on-bumper, lidar-block-slowdown) as the demo
  bootstrap ruleset, so first-run visibly fires.
- **Simulator is a publisher, not a fork**: a `--simulate` source publishes
  synthetic sensor samples on the existing zenoh topics via `Transport::publish`;
  the engine code is untouched and consumes zenoh topics either way.
- **Visible verdict**: when a rule fires, print a loud line so the "aha" is on screen.

## Notes

- Domain: robotics orchestration over Kubernetes — but this map is deliberately
  about the *local, no-infra* experience so anyone can explore the real code.
- Skills every session should consult: `/grilling`, `/domain-modeling`, `/research`.
- Standing preferences: ferrous / no-unsafe is non-negotiable. All demo code is
  safe Rust; no new deps beyond what's present (zenoh loopback is built-in; timer
  is tokio). Reuse existing `Transport`, `RuleStore`, `engine::run_engine`.
- Depends on: all three prior maps (transport classes 1/2, rule engine + hot-reload,
  client-container health/observability, webrtc signaling). This map adds NO new
  architecture — only a simulated input + a friendly front door.
- Issue tracker: local markdown. `Status:` = open/claimed/resolved;
  `Blocked by:` lists ticket numbers that must resolve first.

## Decisions so far

<!-- index — one line per resolved ticket, gist + link to the ticket holding detail -->

- **01 (zenoh loopback config):** `Config::default()` is already a peer with
  multicast scouting on loopback — two `cargo run` on one host auto-mesh with NO
  router, and one process runs standalone. For robustness add
  `listen/endpoints/peer=["tcp/127.0.0.1:0"]` (containers may drop multicast). Set
  programmatically via `Config::insert_json5` (pure safe Rust, no `unsafe`). See
  `issues/01-zenoh-loopback.md` (resolution) + `.scratch/research-zenoh-loopback.md`.

## Not yet specified

- Whether the demo should also **auto-run a second peer** or just document the
  two-terminal recipe — decided during ticket 03 / 04 (onboarding). Lean: document
  the recipe, don't auto-spawn (keeps one command = one node, predictable).
- The **WebRTC signal demo** over loopback is explicitly OUT of this map's scope
  (local demo only was chosen); it can reuse map-03's signaling later.

## Out of scope

<!-- work ruled beyond the destination; closed, never graduates -->

- Kubernetes platform shape (Cilium/Istio), real device access (libudev/camera),
  production DaemonSet hardening — separate efforts/maps.
- The real v4l2->webrtc media pipeline — separate future map.
- These belong to other efforts, not this foolproof-local-demo lock.
