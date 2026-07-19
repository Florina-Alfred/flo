# flo

**`flo` is a Kubernetes robot orchestration client written in safe Rust: sensors and
actuators run in a container, subscribe to data over a Zenoh mesh, and act on it locally
using declarative, hot-reloadable TOML rules.**

## Install & quick start

```bash
cargo install flo-rs     # publishes the binary as `flo`
flo                      # runs the built-in demo on loopback Zenoh
```

`cargo install flo-rs` installs the `flo` binary (the published crate is `flo-rs`; bare
`flo` on crates.io is an unrelated project). Running `flo` with no arguments starts the
local demo: synthetic sensors publish over a loopback Zenoh mesh and the real rule engine
evaluates the two built-in demo rules:

- `e-stop-on-bumper` — bumper pressed AND moving → reliable STOP command.
- `lidar-block-slowdown` — lidar min range < 0.5 m → best-effort slowdown.

No Kubernetes, devices, camera, or config file is required for the demo.

## What you get

- **Zenoh mesh transport** for sensor/actuator traffic (`zenoh` 1.9, unstable features).
- **Hot-reloadable TOML rule engine** — rules live on a Zenoh topic and reload at runtime.
- **Kubernetes fleet coordination** — nodes discover each other and coordinate over the
  same Zenoh mesh (intended for DaemonSet deployment).
- **WebRTC video** (feature-gated) — peer-to-peer video via `webrtc` 0.17, with GStreamer
  encode. Requires the `media` feature **and** a system GStreamer install.
- **`#![forbid(unsafe_code)]`** — every source file in this crate is compiled with the
  `unsafe` code forbidden.
- **Zero system dependencies by default** — the default build is pure Rust (Zenoh +
  webrtc-rs). GStreamer is pulled in only by the `media` feature.
- **CycloneDX SBOM** generated in CI (Security pipeline).

## Examples

Runnable examples live in [`examples/`](examples/). Exact commands (verbatim from the
example headers):

```bash
cargo run --example mesh_demo
#   Run:  cargo run --example mesh_demo
#   Then: cargo run --example mesh_demo -- --robot-id 8
#   Two nodes mesh over loopback Zenoh and fire rules.

cargo run --example custom_rules -- examples/rules/sample.toml
#   Loads a TOML ruleset from a file and reacts to synthetic sensor publishes.

cargo run --features media --example video_peer -- <peer-id>
#   Requires the `media` feature + GStreamer. Without `media` it refuses to build.
```

## Semantic rules (industrial)

Instead of raw Zenoh key-expressions, you can author rules against **zones, roles, poses,
proximity, and human-presence**. `flo` compiles the semantic document to the same runtime rule
engine — no engine change. Authoring is extended TOML (no new dependencies; `#![forbid(unsafe_code)]`
preserved).

```toml
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
```

Validate before deploy:

```bash
flo rule check examples/rules/hrc-cell.toml
```

Semantic `when` keys: `in_zone`, `not_in_zone`, `near_human`, `not_near_human`, `near`,
`role`. Actions: `estop` (reliable STOP), `slow_to(speed)` (best-effort), `resume`. See
`examples/rules/` for an HRC safety cell and a warehouse AMR fleet.

**Safety posture:** `flo` is the software pre-estop / coordination layer. Missing or invalid
config starts `flo` in a fail-safe state (no unrestricted motion commands); pose loss fails
safe. Hardware STO / certified Safety-PLC remains the primary stop authority.

## Configuration / rules

The demo ships with built-in rules. In production mode, pass your own rules file:

```bash
flo --robot-id 7 --config /etc/flo/rules.toml   # production mode (k8s DaemonSet)
```

With no `--config`, `flo` loads an **empty** ruleset (`rules = []`). Rules are TOML; the
rule engine supports composable `when.all` / `when.any` triggers and publishes actions to
Zenoh topics. A sample ruleset (`examples/rules/sample.toml`):

```toml
# Example ruleset for `cargo run --example custom_rules`.
# Fires a STOP when the bumper is pressed, exactly like the built-in demo rule.

[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot/7/local/bumper", pred = "pressed == true" },
  { topic = "robot/7/local/imu",     pred = "speed_mps > 0.2" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
```

Rules can be hot-reloaded at runtime via the Zenoh topic `robot/<id>/local/rules`.

## Security & supply chain

| Property | Status |
| --- | --- |
| `unsafe` code in our crate | Forbidden (`#![forbid(unsafe_code)]`) |
| Default build system deps | None (pure Rust; Zenoh + webrtc-rs) |
| SBOM | CycloneDX, generated in CI (Security pipeline) |
| CI runners | `ubuntu-latest` only (standard, free) |
| Dependency audit | `cargo-audit` hard gate in CI (RUSTSEC) |
| Docker | Skeleton (distroless, non-root) — not yet published |

CI notes (from AGENTS.md and `.github/workflows/security.yml`):

- **Minimal gate** (`ci.yml`) runs on every branch/PR: `fmt`, `cargo clippy -- -D warnings`,
  and a `test` matrix (`stable`, `beta`, `1.97.1`). It is the required status-check gate for
  merging into `main`.
- **Full security pipeline** (`security.yml`) runs only on `main` (push) and `v*` tags:
  `cargo-audit` (RUSTSEC, hard gate with allowlisted advisories in `audit.toml`),
  `cargo-deny` (licenses + banned deps), Trivy filesystem scan (SARIF), and CodeQL (Rust).
- All third-party GitHub Actions are pinned to full commit SHAs.
- The `media` feature is excluded from CI (needs system GStreamer); default features only.
- The Dockerfile is a **skeleton** successfully built but **not** published by CI. It uses a
  distroless base and runs as the non-root `nonroot` user.

## Building from source / development

```bash
cargo build          # default features (no system deps)
cargo run            # runs the built-in demo
cargo test           # runs the test suite
cargo clippy         # lints, -D warnings in CI
cargo fmt            # formats the code
```

The `media` feature requires a system GStreamer install
(`gstreamer`, `gstreamer-app`, `gstreamer-video` 0.25). It is feature-gated so the default
build needs no system libraries.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
