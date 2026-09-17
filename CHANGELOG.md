# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.6] - 2026-08-30

### Added
- `Transport::locators()` and `flo-server`/`flo` now log `zenoh router listening locators=[...]` — the Zenoh mesh port is now discoverable without `ss`/`lsof` (fixes #286 where `health server listening` was mistaken for the Zenoh port).

### Changed
- **Topic seam is now typed:** `topic::robot_local`, `rules_key`, `liveliness_key`, `signal_*`, etc. return `Topic`/`Pattern` newtypes validated at construction (`Topic::try_new`), not `String`. `Transport` is a deep adapter (`open_router`/`connect_to`, single `publish(Topic, Envelope)` + `subscribe(Pattern)`) that hides `zenoh::Config`/`Session`; `auth::zenoh_config` no longer leaks `insert_json5` string interpolation.
- **Registration is two seams:** `RegistrationLease` (state + liveliness token, `on_liveliness_delete` atomic) + `RegistrationTransport` (typed `RegistrationRequest`/`RegistrationResponse`, blocking `register()`). Client retry/backoff is caller policy in `runtime.rs`, not transport.
- **Runtime is one deep module:** `Runtime::bootstrap(Args) -> (Self, ReadyGate)` owns `Transport+ActiveRules+Health` atomically; `ReadyGate` replaces `AtomicBool` + `oneshot` param bag; single `Supervisor` for both binaries (`try_join!` vs `select!` divergence removed, health always supervised).
- **Hot-reload is one module:** `HotReload { transport, store, policy: Option<Registry> }` with single `run()`; `WhenExpr` IR (`All`/`Any`/`Leaf`) with single `lower()` replaces `validate`/`expand_when` dual traversal and `UnrepresentableNesting` duplication.
- **Engine hides wiring:** `EvalState { latest, zones, prev }` owns 50ms tick + ingest + watch-driven rebuild (`store.subscribe()` replaces `sample_count %16`); `ZoneTracker` merged into `EvalState`.
- **Crate surface:** `SemanticDoc` → `RulesManifest`, `RuleStore` → `ActiveRules` (shims kept one release), `Rules` keep; `common.rs` dissolved (`SubsystemHandles`/`start_common_subsystems` → `runtime.rs`, `spawn_video_peer` → `media.rs`, `run_rule_command` → `cli.rs`).
- **README:** Architecture first (§2, diagram + why Zenoh-mesh), then 30-second catch that proves it (§3, `cargo test` + `flo --help` + `rule check` + loopback demo with `--connect` by default, not as a fallback). `FLO_HEALTH_ADDR` vs Zenoh port clarified, `ss`/`lsof` documented as fallback.
- **Docs/tests:** `tests/helpers` (`loopback`, `router_on_free_port`, `wait_for_counter`), `Cargo.toml` `include` now `tests/fixtures/`, `deny.toml` reads `audit.toml` at CI (single source), `flo --print-zenoh-port` trims `verify-readme-demo.sh` (322→79 lines).

### Fixed
- **CI was vacuous:** `ci.yml` `test` ran `cargo test --bin flo` (0 tests) — now `cargo test --lib --tests` + `media --lib --tests` with `Assert tests ran` / `Assert media tests ran`; `cargo test -- --ignored --list` ensures the loopback demo compiles.
- **Crate bloat:** `Cargo.toml` `include` now `38` files (was `157`, leaked `.agents/`, `docs/superpowers/`, `manual_test.log`, `skills-lock.json`, `.actrc`, `deploy/`); `ci.yml` `package` job fails if internals leak.
- **Supply chain:** `Dockerfile` bases pinned by digest (`rust:1.97-slim@sha256:…`, `distroless@sha256:…`, `debian@sha256:…`), `cargo-chef 0.1.71` pinned, `container.yml` SBOM now `amd64`+`arm64` attested, `Publish` gated on `Security` (`workflow_run` + `gate-security` poll), version/tag drift `Cargo.toml == tag` enforced, both `flo` + `flo-server` attached with provenance, `PREV_TAG` SemVer-aware, `cancel-in-progress: false` for releases, `cargo publish` env-only (no `--token` leak), `Cargo.lock` staleness and `CHANGELOG.md` gates added.
- **Safety-critical gaps:** `heartbeat`/`registration` envelope loopback, `hot-reload` conflict/bad TOML, `engine` hot-swap, and `auth` PEM existence/header now have `tests/safety_infra06.rs` + `src/auth.rs` coverage (fixes `engine` `sample_count %16` → `is_multiple_of(16)` and `auth` `validate_pem_file`).
- **README/demo (#286):** `flo-server`/`flo` now log `zenoh router listening locators=[...]`; Quickstart shows the client **with** `--connect` by default and explains `zenoh router listening` vs `health server listening`; `cli_demo_connect` regression asserts the exact `auth.zenoh_config` + `--connect` CLI path that timed out.
- **Stale imports:** `src/engine.rs:12` `Subscription` and `tests/core_loop.rs:4` `AtomicU64` removed so `clippy -D warnings` passes; `src/config.rs`/`engine.rs` `cargo fmt` long lines fixed.

## [0.1.5] - 2026-08-14

### Changed
- Server and client TOML configs now reject unknown fields (`deny_unknown_fields`):
  typos fail fast instead of being silently ignored.
- Migrated the optional `media` feature from `webrtc 0.17` to `webrtc 0.21.0-alpha.1`
  (Sans-I/O API rewrite). Behavior preserved: single H.264 outbound track, host-only
  trickle ICE, receive no-op. Adds `rtc` + `async-trait` as direct deps and drops the
  dormant `gstreamer-video` dependency. Alpha status is a documented risk — tracked
  upstream for a stable 0.21.

## [0.1.4] - 2026-08-13

### Added
- `CHANGELOG.md` and a `keep-a-changelog` discipline for releases.

### Changed
- Bumped crate version to `0.1.4`.
- Added the required `authors` field to `Cargo.toml` (needed for `cargo publish`).

### Fixed
- `CONTRIBUTING.md`: the local `act` example used `act pull_request`, which fails
  locally without a remote base ref; changed to `act push` to match `AGENTS.md`.

> This release aggregates the comprehensive top-to-bottom refinement tracked by
> wayfinder map #165 (the flo-rs 0.1.4 refinement effort). Entries are appended
> here as each fix lands; the `## [Unreleased]` section above collects the next
> cycle's changes.
