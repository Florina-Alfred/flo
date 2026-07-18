# Ticket 01: Research k8s device-access mechanisms for sensors/actuators

Label: `wayfinder:research`
Status: resolved
Blocked by:

## Question

Decide how sensors & actuators are exposed to the DaemonSet container, under the
hard ferrous / no-unsafe constraint (our code) and a sane security posture.

Resolve via a `/research` subagent. Investigate:
- Kubernetes device-exposure options for robotic hardware: `/dev` node mounts via
  `hostPath`, the Device Plugin framework (vendor/community plugins for serial,
  GPIO, USB, CUDA), and `privileged`/`securityContext` trade-offs. Which device
  classes (lidar, camera, IMU, motor controllers) map to which mechanism.
- Non-privileged posture: can most devices be mounted read/write without
  `privileged: true` (e.g. specific `/dev` nodes + `capabilities`)? What the
  minimal `securityContext` looks like.
- Whether any of this touches `unsafe` in OUR Rust code (it shouldn't — device
  access is via filesystem/serial, safe Rust crates like `serialport`, `rscam`/
  `v4l` exist). Flag any crate that would force unsafe on our side.
- Relevance to the transport map's `robot/<id>/local/**` namespace (device I/O is
  local; only decisions/commands cross the mesh).

Capture findings on a throwaway `research/device-access` branch and post a gist +
branch/commit reference as the resolution comment.
