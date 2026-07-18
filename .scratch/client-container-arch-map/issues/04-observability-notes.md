# Observability research: zenoh admin-space + k8s probes + structured logging

Branch: `research/observability`
Status: research findings (AFK subagent). No code changes — notes only.

Hard constraint (from ticket context): this crate is built **ferrous (safety Rust)** and must
contain **NO `unsafe`**. Every recommendation below is verified ferrous-clean.

## 1. Zenoh admin space (`@/`) introspection

Primary source: <https://zenoh.io/docs/manual/abstractions/> (Admin space section).

- The admin space is a dedicated key space under the prefix `@/router/<router-id>`,
  where `<router-id>` is the UUID of a Zenoh router. With the REST API you can use the
  `local` keyword instead of the router UUID.
- Read-only introspection keys (confirmed in docs):
  - `@/<router-id>/router` → JSON status of the router.
  - `@/<router-id>/router/**` → (write-only) runtime config edits.
  - Plugins extend the space, e.g. storage manager adds
    `@/<router-id>/router/status/plugins/storage_manager/...` keys.
- This introspection is router/plugin level. For a **peer/participant** (the DaemonSet
  client), the equivalent is the runtime status of *its own* session. The Rust `Session`
  API does not currently expose a `get_subscribers()`/`get_publishers()` enumerator in the
  public docs; subscriber/publisher inventory is introspectable on `zenohd` routers via the
  admin space, not directly from a peer session. Conclusion: admin-space querying is the
  right signal for **routers/zenohd**, but for a peer participant the robust health signal
  is a **liveliness token**, not polling admin space.

### Recommended zenoh health signal: LivelinessToken

Primary source: <https://docs.rs/zenoh/1.9.0/zenoh/liveliness/> (Rust API).

- `session.liveliness().declare_token("key/expression")` declares a
  `LivelinessToken` whose liveliness is **tied to the Zenoh Session**: when the session
  (process) goes away, the token is automatically undeclared/disappears on the network — no
  explicit cleanup needed.
- Other nodes observe liveness two ways:
  - `session.liveliness().get("key/**")` → query currently-alive tokens.
  - `session.liveliness().declare_subscriber("key/**")` → get `Put` (appeared) /
    `Delete` (lost) events; the `history()` option also replays already-declared tokens.
- Recommendation: each DaemonSet pod declares a unique liveliness token
  (e.g. `robot/<robot-id>/client/liveliness`). A separate watcher (or zenohd + admin/REST)
  subscribes to `robot/**/client/liveliness` to detect dead pods. This is the mesh-native
  health signal and requires zero k8s coupling.

This is the preferred "container health surfaced via zenoh" mechanism from the ticket:
a liveliness token the DaemonSet/operator watches, rather than a probe that queries admin
space.

## 2. k8s liveness/readiness probe design for a zenoh participant

Primary source: <https://kubernetes.io/docs/concepts/workloads/pods/probes/>.

Probe mechanisms available: `exec`, `grpc`, `httpGet`, `tcpSocket`.

Relevant k8s facts (verified):
- `exec` "involves the creation/forking of multiple processes each time when executed...
  in clusters with higher pod densities, lower intervals... might introduce an overhead on
  the cpu usage of the node. In such scenarios, consider using the alternative probe
  mechanisms." → exec is explicitly discouraged for high-density/DaemonSet workloads.
- Common pattern: liveness and readiness share the same low-cost HTTP endpoint, with
  liveness using a higher `failureThreshold`. `httpGet` is first-class; `periodSeconds`
  defaults to 10s, `failureThreshold` defaults to 3.
- A `startupProbe` should protect slow startup; liveness/readiness delays don't begin until
  it succeeds.

### Recommended probe mechanism: HTTP health server (axum) — ferrous-clean

Crate choice: **axum** (0.8.x) on **tokio + hyper**.
Primary sources:
- <https://docs.rs/axum/0.8.9/axum/> — axum is "HTTP routing and request-handling library
  that focuses on ergonomics", built on `tower` and designed to work with `tokio`/`hyper`.
  Its dependency tree is `hyper`, `hyper-util`, `http`, `bytes`, `tower`, `tokio`,
  `tracing` — all well-known safe-Rust crates. No `unsafe` is required by callers; axum
  exposes only safe APIs.
- <https://docs.rs/zenoh/1.9.0/zenoh/liveliness/> confirms zenoh itself already depends on
  `tracing ^0.1` and `tokio ^1`, so pulling `tokio`/`hyper`/`axum` adds no new unsafe
  surface beyond what zenoh already brings.

Why axum over an exec probe:
- Avoids the documented exec-process-fork overhead (matters for DaemonSets on every node).
- A tiny axum router (`Router::new().route("/healthz", get(|| async { "ok" }))`) plus
  `axum::serve(listener, app)` is ~10 lines, all safe Rust, and lets one endpoint serve
  both liveness and readiness (readiness can additionally check the zenoh session +
  liveliness token is declared).
- `hyper`/`hyper-util` are the same HTTP stack zenoh's ecosystem already trusts; both are
  safe Rust (no `unsafe` API forced on our code).

Minimal weight alternative considered: a raw `hyper`-only service avoids axum's routing
abstraction, but axum's ergonomics cost is negligible and it's still 100% safe Rust. Either
is acceptable; **axum is recommended** for clarity.

Conclusion: HTTP `httpGet` probe backed by a tiny axum server is the lightest
ferrous/no-unsafe option; exec probe is explicitly discouraged by k8s docs for dense
DaemonSets.

## 3. Structured logging in safe Rust

Primary sources:
- <https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/> — `tracing-subscriber`
  is "Utilities for implementing and composing `tracing` subscribers." It is MIT licensed,
  no_std-capable, and built entirely on safe Rust (`tracing-core`, `smallvec`,
  `parking_lot`, `regex-automata`, etc.). The `fmt` layer emits human-readable or JSON
  logs; `EnvFilter` gives `RUST_LOG`-style filtering with no unsafe.
- `tracing` is already a **normal dependency of zenoh** (verified in zenoh 1.9.0's
  dependency list), so adopting `tracing` + `tracing-subscriber` on our side adds no new
  crate category and no `unsafe`.

Recommendation: use **`tracing`** for instrumentation and **`tracing-subscriber`** (with the
`fmt` + `env-filter` features; add `json` for structured/JSON log shipping) for the
subscriber. This is the de-facto standard safe-Rust structured-logging stack and is fully
ferrous-clean.

## 4. Does probe/telemetry code force `unsafe` on our side?

No. Summary of unsafe obligations:
- **axum / hyper / hyper-util / tokio / tower**: safe-Rust public APIs; no `unsafe` blocks
  required in application code. (Their transitive deps may use `unsafe` internally, but
  that is not our code and ferrous treats crate-internal `unsafe` as acceptable when the
  crate's public surface is safe — our source stays `unsafe`-free.)
- **tracing / tracing-subscriber / log**: pure safe Rust.
- **zenoh liveliness API**: safe Rust; `LivelinessToken` is tied to the `Session` and
  auto-undeclares on drop.
- Our own code that wires these together (declare liveliness token, spawn axum health
  server, install tracing subscriber) contains **no `unsafe`** and can be compiled under
  `#![forbid(unsafe_code)]`.

## Key recommendation (gist)

- **Zenoh health signal:** declare a per-pod `LivelinessToken` (e.g.
  `robot/<id>/client/liveliness`); watch it mesh-side via `liveliness().declare_subscriber`
  or query via `liveliness().get`. This is the container health signal, automatically lost
  when the process dies.
- **k8s probe:** HTTP `httpGet` probe served by a tiny **axum** (tokio + hyper) health
  server exposing `/healthz` (liveness) and `/readyz` (readiness, also verifies the zenoh
  session + liveliness token). Avoid `exec` (k8s-documented fork overhead at DaemonSet
  density).
- **Logging:** `tracing` + `tracing-subscriber` (fmt + env-filter, +json for shipping).
  Already a zenoh dependency, fully safe Rust.
- **Ferrous:** all three choices are safe-Rust with no `unsafe` obligation on our side;
  mark the crate `#![forbid(unsafe_code)]`.

## Sources
- Zenoh admin space / abstractions: https://zenoh.io/docs/manual/abstractions/
- Zenoh REST plugin (admin-space HTTP access): https://zenoh.io/docs/manual/plugin-http/
- Zenoh liveliness Rust API: https://docs.rs/zenoh/1.9.0/zenoh/liveliness/
- k8s probes concept: https://kubernetes.io/docs/concepts/workloads/pods/probes/
- axum docs.rs: https://docs.rs/axum/0.8.9/axum/
- tracing-subscriber docs.rs: https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/
