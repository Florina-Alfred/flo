# Ticket 01: Research Zenoh deployment topology & ferrous-compatible crate

Label: `wayfinder:research`
Status: resolved
Blocked by:

## Question

Decide how Zenoh is deployed and which Rust crate carries class 1 & 2 traffic,
under the hard ferrous / no-unsafe constraint.

Resolve via a `/research` subagent. Investigate:
- Zenoh deployment topologies applicable to a k8s robot cluster: P2P peer mesh,
  router backbone, and hybrid (local P2P on-node, routers for cross-cluster/edge).
  Trade-offs for a container that decides locally when a network sensor triggers
  an actuator.
- The official `zenoh` Rust crate: its ferrous/safety posture, whether our usage
  path requires `unsafe` on our side, and the QoS knobs it exposes (reliability,
  ordering, durability, history, congestion, deadlines) needed to express class 1
  (reliable/ordered/durable) and class 2 (best-effort/drop-allowed).
- Whether a single zenoh session can multiplex both QoS classes or whether two
  sessions (one per class) are cleaner.

Capture findings on a throwaway `research/zenoh-transport` branch and post a
context pointer (gist + branch link) as the resolution comment.
