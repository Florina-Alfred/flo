# Ticket 02: Build --simulate sensor source

Label: `wayfinder:task`
Status: open
Blocked by: 01

## Question

Task (unblocks the demo): add a `--simulate` source that publishes synthetic sensor
samples on a timer, reusing the existing `Transport::publish` (so the engine, which
subscribes to zenoh topics, is unchanged).

Resolve when: a `simulate` module exists that, on a tokio interval, publishes
samples to the key-exprs the demo rules watch:
- `robot/<id>/local/bumper` with `{pressed: bool}` (toggle/periodic so e-stop fires),
- `robot/<id>/local/imu` with `{speed_mps: f64}`,
- `lidar/fleet/scan` with `{min_range_m: f64}` (dip below 0.5 so slowdown fires).
Use `Transport::publish` with the matching QoS class (bumper/imu = reliable class 1
path key-expr namespace; lidar = best-effort class 2). Pure safe Rust, no devices.
It must not require `libudev` or any hardware. This is the simulated INPUT; the
rule engine is the real, shipped code path.
