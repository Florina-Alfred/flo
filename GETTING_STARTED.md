# Getting Started with `flo`

`flo` is a robot orchestration client: sensors and actuators are wired into a
container that subscribes to sensor data over a [Zenoh](https://zenoh.io) mesh and
acts on it locally using declarative rules. This guide gets you to a working demo
in under two minutes, with **nothing but Rust + cargo**.

## Prerequisites

- Rust toolchain (stable). Check with `cargo --version`.
- That's it. No Kubernetes, no devices, no camera, no config files.

## The one command

```bash
cargo run
```

You'll see a banner, then the rule engine reacting to **simulated** sensors on a
local Zenoh mesh:

```
  flo DEMO  —  robot 7 on loopback zenoh
  Simulating sensors and running the rule engine. Watch for '▶ rule fired'.
  Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)

▶ rule fired  rule=e-stop-on-bumper
▶ published action  rule=e-stop-on-bumper  action=stop/fleet/cmd  qos=Reliable  payload={"stop":true}
▶ rule fired  rule=lidar-block-slowdown
▶ published action  rule=lidar-block-slowdown  action=robot/7/local/drive  qos=BestEffort  payload={"speed_mps":0.1}
```

The simulator publishes synthetic `bumper`, `imu`, and `lidar` samples; the **real**
rule engine (the same code that ships) evaluates the built-in demo rules and fires.
This proves the actual transport + rule-engine path — only the sensor input is fake.

## See two nodes mesh

Open a second terminal and run another node. They auto-discover over loopback Zenoh
(no router, no config):

```bash
# terminal 1
cargo run --robot-id 7
# terminal 2
cargo run --robot-id 8
```

Each node discovers the other via Zenoh presence and can exchange WebRTC signaling
for class-3 video (logged; live media is future work).

## What just happened?

- **Transport** (classes 1 & 2): STOP commands use reliable/ordered/drop-blocked
  Zenoh QoS; lidar uses best-effort/drop-allowed. Class 3 (video) uses WebRTC.
- **Rule engine**: TOML rules with composable `when.all` / `when.any` triggers.
  Hot-reloadable over Zenoh (`robot/<id>/local/rules`).
- **Local actuation**: a node decides locally when a network sensor triggers an
  actuator — no round-trip to a central server required.

## CLI reference

```
cargo run                 # local demo (simulated sensors + built-in rules)
cargo run --robot-id 7    # demo node 7 (mesh with other --robot-id nodes)
cargo run --robot-id 7 --config /etc/flo/rules.toml   # production mode (k8s DaemonSet)
cargo run --simulate --simulate-period-ms 1000        # add fake sensors in any mode
cargo run --help          # full option list
```

## Where to go next

- **Real hardware**: mount devices via a Kubernetes Device Plugin (see
  `deploy/flo-client-daemonset.yaml`) and replace `--simulate` with real `/dev`
  access using safe-Rust crates (`v4l`, `serialport`, ...).
- **Custom rules**: write a TOML ruleset and pass `--config`. Hot-reload by
  publishing new TOML to `robot/<id>/local/rules`.
- **Video**: the WebRTC signaling plane is implemented; attaching v4l2→WebRTC media
  (encoder sidecar) is the next effort.
- **Production**: the DaemonSet manifest runs the same binary as a non-privileged
  pod with health probes and a Zenoh liveliness token.

## Streaming live video (class-3, WebRTC)

`flo` can stream robot camera video peer-to-peer over WebRTC. GStreamer does the
capture + **hardware-accelerated encode** (NVENC `nvv4l2h264enc` on Jetson, `x264enc`
on a dev laptop); webrtc-rs owns the peer connection. Signaling rides the same
zenoh mesh as everything else — no separate service.

Prerequisites (only needed for video):

- System GStreamer >= 1.14 with `x264enc`, `h264parse`, `videotestsrc`
  (apt: `gstreamer1.0-plugins-{base,good,bad,ugly} gstreamer1.0-libav`).
  On Jetson, the NVIDIA accelerated GStreamer packages provide `nvv4l2h264enc`.
- Build with the `media` feature: `cargo build --features media`.

Two terminals, two nodes, real video:

```bash
# terminal 1
cargo run --features media --robot-id 7 --video-peer 8
# terminal 2
cargo run --features media --robot-id 8 --video-peer 7
```

Node 7 captures (synthetic pattern unless `--video-device /dev/video0`), encodes
H.264, and offers a WebRTC call to 8 over zenoh; 8 answers; video flows 7→8.
Node 8 logs `▶ video track received`. No camera? The demo uses `videotestsrc`.

Headless encode check (no peer needed):

```bash
cargo run --features media --video-self-test
```

It builds a GStreamer pipeline against `videotestsrc`, pulls encoded samples, and
asserts valid H.264 (Annex-B start code). Great for verifying Jetson HW encode.

Flags: `--video-peer <id>` (who to call), `--video-device <path>` (real camera;
default = synthetic), `--video-codec h264` (only `h264` in v1), `--video-self-test`.

## Safety note

The crate is built with `#![forbid(unsafe_code)]` and depends only on safe-Rust
crates. `cargo clippy` is clean.
