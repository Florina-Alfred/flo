# Ticket 01: k8s device-access mechanisms for sensors/actuators

Research branch `research/device-access`. Primary sources only (kubernetes.io docs,
docs.rs crate docs, device-plugin project READMEs). No unsafe is required in OUR Rust code.

## 1. Kubernetes device-exposure options

### A. hostPath `/dev` node mounts
- A `hostPath` volume pointed at a single device node (e.g. `/dev/ttyUSB0`) or the
  whole `/dev` directory, mounted read/write into the container.
- Simplest, no extra controllers. Trade-offs:
  - Mounting all of `/dev` is effectively privileged-equivalent and trips Pod Security
    Standards `restricted` (and often `baseline`). Point-to-node mounts are better.
  - Device node major/minor is fixed at *host* paths; pod must tolerate the node that
    has the hardware (nodeSelector / nodeAffinity / taint).
  - No sharing/accounting: two pods mounting the same physical device both get it.
- Best for: fixed, node-local, always-present devices with a stable path.

### B. Device Plugin framework
- Per k8s docs (https://kubernetes.io/docs/concepts/extend-kubernetes/compute-storage-net/device-plugins/):
  vendors implement a `DevicePlugin` gRPC service; deploy as a DaemonSet. The plugin
  advertises extended resources to kubelet; consumer pods request them as
  `resources.limits.<domain>/<name>: "1"`. On Allocate, kubelet mounts the device file
  into the pod's `/dev` (or a `mountPath`).
- The *plugin* pod runs privileged (it mounts `/var/lib/kubelet/device-plugins`, which
  the doc says requires privileged access). **The consumer pod does NOT need to be
  privileged** — the device is injected by kubelet at container start.
- Community/vendor plugins relevant to robotics (from k8s docs "device plugin
  implementations" list + project READMEs):
  - **squat/generic-device-plugin** — generic Linux devices via `--device` config:
    serial (`/dev/ttyUSB*`, `/dev/ttyACM*`), video (`/dev/video0`), sound
    (`/dev/snd/...`), FUSE, and **USB by Vendor/Product ID** (e.g. CH340
    `1a86:7523`). Grouped/optional/per-count allocation. Advertises as `devic.es/<name>`.
  - **smarter-project/smarter-device-manager** — Raspberry-Pi-class IoT; regex-discovers
    V4L, I2C, SPI, sound, etc. and adds them as kubelet resources.
  - **Akri** — auto-discovers leaf devices (IP cameras via ONVIF, USB via udev, OPC-UA)
    and brokers pods when a device appears.
  - Vendor: **NVIDIA GPU plugin** (CUDA), Intel FPGA/GPU/QAT, SocketCAN, RDMA, SR-IOV.
- Best for: dynamic/heterogeneous fleets, device pooling, USB-by-ID targeting, when you
  want scheduling + accounting and a clean non-privileged consumer posture.

### C. Device class → mechanism mapping
| Device class | Examples | Recommended mechanism |
|---|---|---|
| Serial / UART | lidar (USB-UART), motor controllers, IMU via tty | generic device plugin (serial + USB-by-ID); or hostPath `/dev/tty*` if fixed |
| USB (vendor-specific) | CH340/FTDI converters, UVC cams | generic device plugin USB-by-Vendor/Product-ID (`/dev/serial/by-id`) |
| Camera / v4l2 | `/dev/video0`, class-3 video | generic device plugin `video`; hostPath `/dev/videoN` if fixed |
| GPIO / I2C / SPI | sysfs GPIO, `/dev/i2c-*`, `/dev/spidev*` | generic device plugin (path groups) or hostPath node mount |
| CUDA / GPU | accelerators for vision ML | NVIDIA device plugin |
| NIC / FPGA / RDMA | high-speed interconnects | vendor plugin (Intel, SR-IOV, RDMA) |

## 2. Minimal non-privileged securityContext

Device *file* access (read/write on a char device) needs only filesystem permissions on
the node — **no Linux capability, and no `privileged: true`**. The Talos generic-device-plugin
guide shows the canonical consumer pod:

```yaml
securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop: [ALL]
    # add only what the workload genuinely needs, e.g.:
    # - NET_ADMIN   # only if doing raw networking
    # - SYS_RAWIO   # only for rare ioctl-heavy devices
```

Minimal posture for our DaemonSet:
- `privileged: false`
- `allowPrivilegeEscalation: false`
- `capabilities.drop: [ALL]` (add none unless a specific device ioctl demands it)
- `readOnlyRootFilesystem: true` (with writable volume for logs/state)
- `runAsNonRoot: true`, `seccompProfile: RuntimeDefault`, `appArmorProfile: RuntimeDefault`
- Devices arrive via the device plugin (or explicit hostPath mounts), not via `privileged`.

Caveat: `SYS_RAWIO`/`SYS_ADMIN` are sometimes needed for exotic memory-mapped or
bus-probing hardware; pick per device. Standard serial/v4l2/gpio/i2c do NOT need them.

## 3. Does this force `unsafe` in OUR Rust code?

No. Device access is via filesystem/serial IO; safe-Rust crates provide the bindings.
Verified on docs.rs (all expose safe APIs; we write no `unsafe`):

- **serialport** (4.9.0, MPL-2.0) — cross-platform serial; `SerialPort` trait,
  `TTYPort` on Unix; opens `/dev/tty*`. Pure safe API.
- **v4l** (0.14.0, MIT) — "Safe video4linux (v4l) bindings"; `Device::new(index)`,
  `MmapStream`/`UserPtrStream` capture from `/dev/videoN`. Safe API.
- **i2cdev** (0.6.2, MIT/Apache-2.0) — "safe access to Linux i2c device interface";
  `LinuxI2CDevice::new("/dev/i2c-1", addr)`, SMBus + `transfer` API. Safe API.
- **sysfs_gpio** (0.6.2, MIT/Apache-2.0) — GPIO via Linux sysfs; `Pin::new(n)`. Safe API.

None of these require `unsafe` in application code. (Under the hood they wrap `libc`/
syscalls, but that unsafe lives inside the crate, not our `flo` source — consistent with
the ferrous / no-unsafe constraint on *our* code.) `rscam` is deprecated in favor of `v4l`.

## 4. Relevance to the transport design

Device I/O is strictly local (filesystem/serial inside the container). It does NOT cross
the zenoh mesh. This matches the locked transport map:
- Sensor reads and actuator writes happen in-container via the rule engine.
- Only **decisions/commands** (`robot/<id>/local/**`) and telemetry cross zenoh.
- The device-access mechanism (plugin vs hostPath) is a *deployment/packaging* concern
  and is invisible to the transport layer — the rule engine just opens `/dev/...` paths.

## Recommendation

Use the **generic device plugin** (squat/generic-device-plugin or smarter-device-manager)
as a DaemonSet to advertise serial/USB/video/I2C/GPIO devices, and mount them into our
non-privileged `flo` DaemonSet pods (`capabilities.drop: ALL`, `allowPrivilegeEscalation:
false`). Fall back to explicit `hostPath` `/dev` mounts only for fixed, always-present
nodes where a plugin is overkill. Keep device I/O local; publish only commands/telemetry
to zenoh.

Sources:
- https://kubernetes.io/docs/concepts/extend-kubernetes/compute-storage-net/device-plugins/
- https://kubernetes.io/docs/tasks/configure-pod-container/security-context/
- https://github.com/squat/generic-device-plugin
- https://github.com/smarter-project/smarter-device-manager
- https://docs.talos.dev/kubernetes-guides/advanced-guides/device-plugins/
- https://docs.rs/serialport, https://docs.rs/v4l, https://docs.rs/i2cdev, https://docs.rs/sysfs_gpio
