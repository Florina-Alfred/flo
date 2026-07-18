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

## Safety note

The crate is built with `#![forbid(unsafe_code)]` and depends only on safe-Rust
crates. `cargo clippy` is clean.
