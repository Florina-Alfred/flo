# Ticket 04: Research zenoh admin-space telemetry + k8s probe integration

Label: `wayfinder:research`
Status: open
Blocked by:

## Question

Decide how the container exposes observability: k8s probes + zenoh admin-space
telemetry + structured logs, and how they hook together.

Resolve via a `/research` subagent. Investigate:
- Zenoh admin space (`@/` queries): what introspection is available (sessions,
  subscribers, publishers, liveliness tokens) and how to surface container health
  (e.g. a liveliness token the DaemonSet can watch, or a probe that queries admin
  space).
- k8s liveness/readiness probe design for a zenoh participant: an HTTP endpoint
  backed by safe Rust (e.g. a tiny `axum`/`hyper` health server) vs an exec probe.
  Prefer the lightest ferrous/no-unsafe option.
- Structured logging in safe Rust (tracing/log crates) — no unsafe concern, just
  confirm the crate choice.
- Whether probe/telemetry code forces `unsafe` on our side (it shouldn't).

Capture findings on a throwaway `research/observability` branch and post a gist +
branch/commit reference as the resolution comment.

## Resolution (research findings)

Branch `research/observability`, commit `9a6ed53` (notes:
`.scratch/client-container-arch-map/issues/04-observability-notes.md`). All recommendations
verified ferrous-clean (no `unsafe` obligation on our side).

- **Zenoh health signal:** declare a per-pod `LivelinessToken`
  (`robot/<id>/client/liveliness`) via `session.liveliness().declare_token(...)`; watch
  mesh-side with `liveliness().declare_subscriber("robot/**/client/liveliness")` or query
  with `liveliness().get(...)`. The token is tied to the `Session` and auto-disappears when
  the process dies — no explicit cleanup. Admin-space (`@/router/<id>`) introspection is
  router-level; for a peer participant a liveliness token is the right signal, not polling
  admin space.
- **k8s probe:** HTTP `httpGet` probe backed by a tiny **axum** (tokio + hyper) health
  server exposing `/healthz` (liveness) and `/readyz` (readiness, also verifies the zenoh
  session + liveliness token). Avoid `exec` — k8s docs warn of process-fork CPU overhead at
  DaemonSet pod density. axum/hyper are safe Rust; zenoh already depends on tokio + tracing.
- **Logging:** `tracing` + `tracing-subscriber` (fmt + env-filter, +json for shipping).
  Already a zenoh dependency; fully safe Rust.
- **Ferrous confirmation:** none of axum/hyper/tokio/tracing/tracing-subscriber/zenoh
  liveliness force `unsafe` in our code. Mark the crate `#![forbid(unsafe_code)]`.

Sources: zenoh abstractions + REST plugin + liveliness API docs; k8s probes concept page;
axum and tracing-subscriber docs.rs. See notes file for URLs.
