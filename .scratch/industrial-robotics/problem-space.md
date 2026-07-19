# Industrial Robotics Problem Space — Research for `flo`

Primary-source research into the pain points a safe-Rust / Zenoh / Kubernetes-DaemonSet
robot orchestration client could address. Every non-obvious claim is cited to a primary or
first-party source.

---

## 1. Real-time / latency pain in robot fleets

- **DDS retransmission is blind to transport fragmentation.** In lossy wireless networks a single large ROS 2 message split across many UDP packets triggers a full retransmit of all packets when one is lost, causing burst traffic and queue saturation. Practitioners lack a methodology to predict this latency. — [arXiv:2508.10413](https://arxiv.org/html/2508.10413v1)
- **ROS 2 adds up to ~50% latency overhead over raw DDS** at default settings; end-to-end latency is highly sensitive to which DDS middleware (FastDDS/CycloneDDS/Connext) and hardware is used, and to NIC/CPU power-saving features. Tuning is non-obvious. — [Barkhausen Institut latency study (2021)](https://www.barkhauseninstitut.org/fileadmin/user_upload/Publikationen/2021/2021_Kronauer_Latency.pdf)
- **DDS discovery does not scale.** Its discovery protocol is O(Topics·Readers·Writers·Participants²), reliability resource usage scales with matching readers, and it floods UDP multicast on wireless/WAN — unsuitable for open or Internet-scale comms. — [ROScon 2023, "Why DDS Cannot Scale?" (eProsima/Zenoh)](https://roscon2023.de/presentations/S4_P4___ROS_2_Kommunikationsoptimierung_mit_Zenoh-Bridge-DDS.pdf)
- *Why it hurts:* latency-critical STOP/estop across many nodes is fought with QoS knobs whose failure modes are poorly understood; the de-facto standard (ROS 2/DDS) is weakest exactly where fleets live — wireless, lossy, wide-area.

## 2. Safety & estop distribution at fleet scale

- **Standards mandate a reliable, independent estop.** ISO 10218-1:2025 requires every robot to have an independent emergency stop with priority over all other controls, removing actuator power, remaining active until reset. ISO 13849 assigns e-stop a required Performance Level (PLr) often **PLe / Category 4** (dual-channel, continuous cross-fault monitoring) for a standard cell. — [ISO 10218-2:2025](https://www.iso.org/standard/73934.html), [ISO 13849-1:2023](https://cdn.standards.iteh.ai/samples/73481/53161c0051c842dfa32a139fd0729a4c/ISO-13849-1-2023.pdf), [GT-Engineering 10218 stop functions](https://www.gt-engineering.it/en/technical-standards/en-iso-standards/en-iso-10218-1-safety-requirements-for-industrial-robots/5-5-robot-stopping-functions/)
- **Centralized Safety PLC vs distributed.** A central Safety PLC simplifies validation/proof-test but a single controller coordinating 100s of nodes becomes the validation bottleneck and a single point of failure for interlocks; distributed local safety logic is harder to validate and extend. — [ICNavigator Safety PLC topology](https://icnavigator.com/applications/industrial-robotics/safety-controller-plc/)
- **Software estop is necessarily secondary to hardware.** A software layer depends on the scheduler/process being alive; the primary safety system is a hardware relay/STO that cuts power independently. — [HORUS estop recipe](https://docs.horusrobotics.dev/recipes/emergency-stop)
- *Why it hurts:* propagating a certified estop to 100s of nodes fast, with fail-safe semantics (no-signal = fault) and validation across the fleet, is not solved by general-purpose pub/sub.

## 3. Kubernetes at the edge / factory floor

- **Vanilla k8s assumes always-on, routable control-plane contact.** Edge nodes behind flaky WAN get marked `NotReady`, pods evicted, workloads rescheduled — wrong behavior when "elsewhere" doesn't exist. KubeEdge/OpenYurt change this with offline autonomy. — [KubeEdge architecture](https://kubeedge.io/), [Giant Swarm edge mismatch analysis](https://dorland.org/kubeedge-extends-kubernetes-to-unreliable-resource-limited-edge-sites-learn-architecture-device-twins-security-and-how-it-compares-to-k3s-in-production/)
- **KubeEdge offline survival is real but has gaps:** `kubectl logs/exec` fail when the tunnel is partitioned; MetaManager SQLite can wedge on power loss mid-write; observability is lost exactly when the cloud link drops. — [KubeEdge in Production (2026)](https://iotdigitaltwinplm.com/kubeedge-production-deep-dive-tutorial-2026/)
- **Device heterogeneity is first-class pain.** Akri extends the device-plugin framework to "leaf devices" (IP cameras, USB, OPC UA) with dynamic appear/disappear; vanilla k8s has no concept of a device as an object. — [Akri (CNCF Sandbox)](https://github.com/project-akri/akri)
- *Why it hurts:* running robotics on k8s means reconciling "keep running locally when partitioned" with "I still need to actuate and observe."

## 4. Middleware fragmentation & what Zenoh improves

- **Too many middlewares because each optimizes a different slice:** DDS (real-time, QoS, wired LAN), MQTT/Kafka (brokered, cloud), OPC UA (industrial PLC/MES). ROS 2 adopted DDS (Tier-1: CycloneDDS/FastDDS/Connext). — [ROS 2 DDS overview, TU Dortmund](https://daes.cs.tu-dortmund.de/storages/daes-cs/r/publications/teper_rtss_2022.pdf)
- **Zenoh: 5-byte minimal overhead, peer-to-peer OR routed, runs from MCU to datacenter, over TCP/UDP/QUIC/serial/BT.** It cut DDS discovery traffic by 97–99% and reached ~2× DDS / 50–100× MQTT-Kafka throughput; outperforms DDS on Wi-Fi/4G (where DDS multicast floods). — [Zenoh vs MQTT/Kafka/DDS (NTU, arXiv:2303.09419)](https://arxiv.org/pdf/2303.09419), [CycloneDDS RMW report](https://osrf.github.io/TSC-RMW-Reports/humble/eclipse-cyclonedds-report.html)
- Zenoh is the **first non-DDS protocol natively supported in ROS 2** (rmw_zenoh), targeting the "walled garden" interop problem. — [ScienceDirect Zenoh IIoT survey](https://www.sciencedirect.com/science/article/pii/S1570870525000320)
- *Why it hurts:* fleets need one protocol that is low-footprint on embedded, peer-to-peer on the floor, and routeable to the cloud — which is exactly Zenoh's design center.

## 5. OTA / config hot-reload / fleet config drift

- **Manual config = "snowflakes."** Past ~10 robots, hand-edited YAML/configs cause misconfigurations, downtime, and untraceable who-changed-what. A fleet of 100 can mean 100 config repos. — [Miru "config is a mess"](https://mirurobotics.substack.com/p/robotics-config-management-is-a-mess)
- **Push-based OTA is brittle at the edge:** Ansible/Compose are push models unaware of device state; network drops mid-update leave bad states; no atomicity, no easy rollback. — [Miru K3s+ArgoCD OTA](https://mirurobotics.substack.com/p/using-k3s-and-argocd-for-robotics)
- **Bootloader-stage OTA failures brick robots** (non-atomic A/B, shared poisoned state, fleets go down fleet-wide in minutes). Atomic/reconciler pull models with signed artifacts + local safe-state are the emerging answer. — [Why OTA bricks fleets](https://tech-champion.com/robotics/why-ota-firmware-updates-brick-autonomous-robot-fleets/), [Markaicode safe OTA](https://markaicode.com/ota-updates-robots-safe-software-deployments/)
- *Why it hurts:* logic/rule changes must reach 100s of nodes, hot, with rollback and audit trail — without a cloud round-trip or a reboot.

## 6. Video / telemetry observability gap

- **Industrial video is stuck in vendor silos / RTSP+NVR / VMS.** ONVIF (RTSP, Profile S/T/M) is the integration lingua franca for fixed cameras but does not cover robot routing/coordination; proprietary stacks force parallel operator UIs. — [Quarero ONVIF robotics](https://quarerorobotics.com/blog/en-sicherheitsroboter-leitstelle-onvif), [iFovea cloud VMS](https://www.ifovea.com/)
- **RTSP/HTTP video is wrong for mobile robots:** 500–2000 ms latency with buffering, needs VPN/port-forward, no NAT traversal, no adaptive bitrate. WebRTC gives 100–300 ms, browser-native, DTLS-SRTP, ICE NAT traversal — but is "still a hell of a task to implement on an end device (not a browser)." — [Transitive Robotics WebRTC](https://transitiverobotics.com/blog/streaming-video-from-robots/), [Fictionlab WebRTC on robots](https://fictionlab.pl/blog/webrtc-on-robots-how-to-stream-live-video-from-your-rover-to-any-browser/)
- **Reaching robots behind customer firewalls is an unsolved, unpublished pain**; no vendor productized end-to-end. — [automaton.run "reaching robots behind firewalls"](https://automaton.run/post/reaching-robots-behind-customer-firewalls)
- *Why it hurts:* getting live operator video peer-to-peer without a central server, on embedded, encrypted, behind NAT, is exactly the unsolved gap.

## 7. Supply-chain / safety-critical software hygiene

- **Rust's compiler does ~90% of what stack-analysis/MISRA enforced manually** — a "breakthrough" for safety-rated systems — but ecosystem thins at higher integrity: no qualified compiler historically, dependency drift, `no_std` target caveats. — [Rust blog: shipping Rust in safety-critical (2026)](https://blog.rust-lang.org/2026/01/14/what-does-it-take-to-ship-rust-in-safety-critical/)
- **At ASIL B+/SIL 3+, third-party crates become hard to justify**; teams rewrite/internalize critical crates. Single-binary, zero-dependency design directly answers this. — [ibid.], [Safety-Critical Rust Consortium](https://rustfoundation.org/media/announcing-the-safety-critical-rust-consortium/)
- **`unsafe`/build-scripts/proc-macros are the real supply-chain risk** even in Rust; `cargo-audit`/`cargo-deny`/`cargo-geiger` + SBOM (EU CRA) are the expectation. `flo`'s `#![forbid(unsafe_code)]` + no deps is the strongest possible posture here. — [Safeguard Rust supply-chain guide](https://safeguard.sh/resources/blog/rust-cargo-dependency-security-guide), [Rust embedded supply chain](https://safeguard.sh/resources/blog/rust-embedded-supply-chain-guide)
- *Why it hurts:* certifiers ask for provenance, SBOM, and a bounded `unsafe` surface; most robotic stacks are C++ with large, unaudited dependency trees.

---

## Ranked shortlist — where `flo` is uniquely well-positioned

**1. Distributed, peer-driven local actuation rules (estop / protective-stop class).**
`flo`'s hot-reloadable TOML `when.all/any → actions` over a Zenoh mesh gives reliable/ordered STOP (QoS class) and best-effort lidar, processed locally on the node with the hardware — no central server round-trip, fail-safe by design. This hits areas 1, 2, 5. *Smallest credible increment:* a signed, versioned TOML rule-set delivered via Zenoh with a local "safe-state on bad/missing config" fallback, validated against a fleet canary. (Not a certified Safety-PLC replacement — positions as the fast software pre-estop / coordination layer per HORUS framing.)

**2. Fleet config / logic hot-reload without OTA reboot or drift.**
Declarative TOML rules pushed over Zenoh, hot-reloaded, with version tracking, beats manual YAML (area 5). *Smallest increment:* Zenoh-keyed rule config + `flo` watch + atomic swap + rollback to last-good, all single-binary, no container restart.

**3. Peer-to-peer WebRTC video + telemetry to an operator, no central server.**
`flo` already plans GStreamer H.264 WebRTC with Zenoh signaling (area 6). This solves the "reach the robot behind a firewall / no VMS" gap using Zenoh for NAT-traversal-friendly signaling. *Smallest increment:* Zenoh-based SDP/ICE signaling channel + one `webrtcbin` pipeline; SFU later if multi-viewer needed.

**4. Zero-dependency, `forbid(unsafe)` safety-critical hygiene baseline.**
Single binary, no deps, memory/thread-safe by construction — directly answers area 7's certifier demands (SBOM trivial, no `unsafe`, no abandoned-crate risk). *Smallest increment:* ship `cargo-deny`/`cargo-audit` gates + CycloneDX SBOM in CI; document the `unsafe` surface (zero).

**5. Kubernetes-native edge actuation that survives partition.**
As a DaemonSet co-located with hardware, `flo` keeps acting locally when the control plane is unreachable (areas 3, 5) — k8s probes + Zenoh liveliness for observability, device plugins for `/dev`. *Smallest increment:* DaemonSet + liveness/readiness probes + local-safe-state on partition, no cloud dependency for actuation.

## Not a good fit for `flo` (architecture mismatch — avoid)

- **Heavy path-planning / SLAM / ML perception training.** `flo` is an orchestration/actuation client, not a compute engine; these need GPU/classical-planning stacks (ROS 2 Nav2, Isaac). — cf. [ROS 2 navigation/SLAM scope](https://doi.org/10.31224/6508)
- **Certified functional-safety (SIL/PL) stop authority.** `flo` is software-only; ISO 13849/61508 stop functions require hardware STO/relay + certified toolchain (Ferrocene). `flo` can be the *pre-estop*/coordination layer, not the certifiable safety element. — [ISO 13849-1:2023](https://cdn.standards.iteh.ai/samples/73481/53161c0051c842dfa32a139fd0729a4c/ISO-13849-1-2023.pdf), [Ferrocene/Sonair case study](https://ferrous-systems.com/pdf/sonair-case-study-2026-03.pdf)
- **Cloud-centric fleet management / multi-tenant control planes.** `flo`'s thesis is peer-driven local actuation; central fleet OTA/telemetry backends (ArgoCD, VMS) are complementary, not in-scope.
- **Real-time kernel / deterministic microsecond scheduling.** `flo` runs as a non-privileged pod; hard RT is the node OS / safety MCU's job, not the orchestration client's.
