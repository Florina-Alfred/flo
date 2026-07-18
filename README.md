# flo

**Kubernetes robot orchestration in safe Rust — with a peer-to-peer WebRTC video pipeline.**

`flo` is a robot orchestration client: sensors and actuators live in a container that
subscribes to sensor data over a [Zenoh](https://zenoh.io) mesh and acts on it locally
using declarative rules. Transport classes 1 & 2 (STOP = reliable/ordered, lidar =
best-effort) ride Zenoh; class 3 (video) rides WebRTC. All of it is written in safe Rust
with `#![forbid(unsafe_code)]`.

[![CI](https://github.com/Florina-Alfred/flo/actions/workflows/ci.yml/badge.svg)](https://github.com/Florina-Alfred/flo/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Rust 1.97.1+](https://img.shields.io/badge/rust-1.97.1%2B-orange.svg)

## Requirements

- **Rust:** >= 1.97.1 (MSRV). Edition 2024.
- **Operating systems:** Linux, macOS, or Windows (standard GitHub runners).
- **Video only:** system GStreamer >= 1.14 with `x264enc`/`h264parse`/`videotestsrc`
  (and `nvv4l2h264enc` on Jetson) plus the `media` feature:
  `cargo build --features media`.

## Quick start (the one command)

```bash
cargo run
```

You get a banner and the rule engine reacting to **simulated** sensors on a local Zenoh
mesh in under two minutes — no Kubernetes, no devices, no camera, no config files.

```
  flo DEMO  —  robot 7 on loopback zenoh
  Simulating sensors and running the rule engine. Watch for '▶ rule fired'.
  Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)

▶ rule fired  rule=e-stop-on-bumper
▶ published action  rule=e-stop-on-bumper  action=stop/fleet/cmd  qos=Reliable  payload={"stop":true}
```

The simulator publishes synthetic `bumper`, `imu`, and `lidar` samples; the **real** rule
engine (the same code that ships) evaluates the built-in demo rules and fires. This proves
the actual transport + rule-engine path — only the sensor input is fake.

## See two nodes mesh

```bash
# terminal 1
cargo run --robot-id 7
# terminal 2
cargo run --robot-id 8
```

Each node discovers the other via Zenoh presence and can exchange WebRTC signaling for
class-3 video.

## CLI reference

```
cargo run                 # local demo (simulated sensors + built-in rules)
cargo run --robot-id 7    # demo node 7 (mesh with other --robot-id nodes)
cargo run --robot-id 7 --config /etc/flo/rules.toml   # production mode (k8s DaemonSet)
cargo run --simulate --simulate-period-ms 1000        # add fake sensors in any mode
cargo run --help          # full option list
```

## Streaming live video (class-3, WebRTC)

`flo` streams robot camera video peer-to-peer over WebRTC. GStreamer does capture +
**hardware-accelerated encode** (NVENC `nvv4l2h264enc` on Jetson, `x264enc` on a dev
laptop); webrtc-rs owns the peer connection. Signaling rides the same Zenoh mesh as
everything else — no separate service.

Two terminals, two nodes, real video:

```bash
# terminal 1
cargo run --features media --robot-id 7 --video-peer 8
# terminal 2
cargo run --features media --robot-id 8 --video-peer 7
```

Node 7 captures (synthetic pattern unless `--video-device /dev/video0`), encodes H.264,
and offers a WebRTC call to 8 over Zenoh; 8 answers; video flows 7→8. Each node offers
and answers, so video flows both ways once both are up. The `▶ video track received` line
appears on the receiving side. No camera? The demo uses `videotestsrc`.

Headless encode check (no peer needed):

```bash
cargo run --features media --video-self-test
```

Flags: `--video-peer <id>`, `--video-device <path>` (default = synthetic),
`--video-codec h264` (only `h264` in v1), `--video-self-test`.

## Architecture

- **Transport (classes 1 & 2):** Zenoh QoS — STOP commands reliable/ordered/drop-blocked;
  lidar best-effort/drop-allowed.
- **Rule engine:** TOML rules with composable `when.all` / `when.any` triggers,
  hot-reloadable over Zenoh (`robot/<id>/local/rules`).
- **Local actuation:** a node decides locally when a network sensor triggers an actuator —
  no round-trip to a central server.
- **Class 3 (video):** WebRTC peer connection (webrtc-rs) with GStreamer H.264 encode,
  signaling over Zenoh.

## Security & Quality

This project takes safety and supply-chain hygiene seriously:

- **Zero `unsafe`:** the crate is compiled with `#![forbid(unsafe_code)]`. Our code contains
  no `unsafe` blocks; any `unsafe` in dependencies is confined to well-audited crate-internal
  FFI (e.g. webrtc/gstreamer C bindings). We verify the claim with `cargo-geiger`-style review
  and keep the dependency surface to the safe-Rust ecosystem.
- **Minimal, vetted dependencies:** no dependencies beyond the standard safe-Rust ecosystem.
  `openh264` was explicitly rejected (we use GStreamer-native encode only). GStreamer is
  feature-gated behind `media` so the default build needs no system libraries.
- **Continuous security scanning:** every push and PR runs, on free standard runners:
  - `cargo-audit` — RUSTSEC advisory check.
  - `cargo-deny` — license and banned-dependency policy enforcement.
  - `Trivy` — filesystem vulnerability, secret, and misconfiguration scan.
  - `CodeQL` — semantic code-analysis for the Rust language.
  SARIF/report artifacts are uploaded on every run (30-day retention).
- **Strict linting:** `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
  gate every change. Convention: avoid magic numbers (named constants / documented literals);
  enforced via strict clippy and review.

See `docs/superpowers/specs/` for detailed design documents.

## Where to go next

- **Real hardware:** mount devices via a Kubernetes Device Plugin
  (`deploy/flo-client-daemonset.yaml`) and replace `--simulate` with real `/dev` access.
- **Custom rules:** write a TOML ruleset and pass `--config`; hot-reload via Zenoh.
- **Production:** the DaemonSet manifest runs the same binary as a non-privileged pod with
  health probes and a Zenoh liveliness token.
