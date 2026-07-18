# Ticket 03 — k8s P2P WebRTC Connectivity (STUN/TURN vs mesh)

Research branch: `research/webrtc-connectivity`
Status of ticket: open (research; does NOT block ticket 01).

## TL;DR recommendation

- **Primary ICE strategy: host candidates only (UDP), no STUN/TURN required** for
  same-cluster pod-to-pod video on Cilium (and any CNI that gives pods routable
  pod IPs — i.e. the whole k8s network model). Direct P2P "just works" at L3/L4.
- **No signaling-infra change**: TURN is a *media* relay, not a signaling service.
  Adding coturn does not break the "zenoh-only signaling" decision.
- **Scope a TURN relay only for the explicitly-future cases**: cross-cluster robot
  meshes, and browser viewers behind NAT/firewall. Keep it out of the v1 path;
  make ICE server config (STUN/TURN URLs) pluggable so it can be added later
  without touching the signaling schema.
- **Envelope**: ticket 01's signaling envelope must carry SDP + ICE candidates
  opaquely (Trickle ICE). It does NOT need to know about host-vs-relay — but the
  `RTCPeerConnection` config (iceServers) is a deployment-time secret/config, not
  part of the envelope. Envelope stays transport-agnostic as already designed.
- **Ferrous guarantee**: none of this touches `unsafe` in our code. webrtc-rs handles
  ICE; our code only serializes SDP/candidates over zenoh. Confirmed: no WebRTC
  client code exists in `src/` yet — this is pure infra/config.

---

## 1. Pod-to-pod WebRTC reachability in CNI meshes

### Kubernetes network model (the baseline contract)
- Every pod gets its own IP; **all pods can reach all other pods directly by IP
  without NAT**; nodes reach pods without NAT; the pod IP is the same inside and
  out. This is the CNI contract every plugin (Cilium, Calico, Flannel, Istio's
  CNI) fulfills. [kubernetes.io/docs/concepts/cluster-administration/networking/;
  Calico "Kubernetes network model"; kubemastery crash-course]
- Consequence for ICE: `host` candidates (pod IP + UDP port) are directly
  reachable between any two pods in the cluster, on any node. No NAT traversal,
  no reflexive address needed. ICE host↔host connectivity check succeeds.

### Cilium (eBPF, direct pod networking)
- "In the absence of any network security policies, all Pods can reach each
  other." Each pod gets an IP from the node prefix; pod IPs are local to the
  cluster but fully routable within it. [docs.cilium.io/en/stable/network/kubernetes/intro/]
- Cilium enforces L3/L4 (and optional L7) policy, but **does not proxy or NAT
  pod-to-pod traffic** — it programs eBPF maps. So WebRTC UDP host candidates
  traverse directly. (If `CiliumNetworkPolicy` restricts it, that's an explicit
  allowlist, not a blocker.)
- **Same cluster: direct P2P, no TURN. Cross-cluster: only via ClusterMesh
  (same L2/L3 fabric) or a relay; otherwise NAT/firewall between clusters breaks
  host candidates → needs TURN.**

### Istio (sidecar / Envoy)
- Sidecar model: Envoy intercepts traffic via iptables; routing is keyed on
  **Service IP**, not pod IP. "Direct calls to pods (curl <POD_IP>), rather than
  Services, will not be matched. While the traffic may be passed through, it will
  not get the full Istio functionality including mTLS." [istio.io/docs/ops/deployment/application-requirements/]
- Key point: a pod-to-pod **UDP P2P media stream addressed to the remote pod IP
  is passed through at L3/L4**; it simply bypasses Envoy's L7/mTLS/routing. That
  is exactly what we want for WebRTC media — we do NOT want Istio mTLS-terminating
  our RTP. So Istio does **not** block direct host-candidate P2P; it just doesn't
  "help" it.
- Caveat: if a `Sidecar`/`DestinationRule` or strict egress policy blocks
  arbitrary pod-egress UDP, host candidates can be filtered — that's a policy
  knob, not inherent. Ambient mode (ztunnel/HBONE) similarly tunnels TCP/HTTP;
  raw UDP media to pod IPs is passed through.
- **Same cluster: direct P2P works (host candidates pass through). Cross-cluster
  (multi-primary / different networks): needs TURN.**

**Bottom line q1:** Direct P2P between pods on the same cluster works without a
TURN relay on both Cilium and Istio. Cross-cluster requires TURN (or a mesh that
extends pod routability, e.g. ClusterMesh on a shared underlay).

---

## 2. STUN vs TURN — when each is needed, minimal coturn, zenoh-spirit

### Definitions (primary: RFC 5389 STUN, RFC 5766 TURN; webrtc docs)
- **STUN** (RFC 5389): lightweight; a client discovers its public IP:port as seen
  from outside. Produces `srflx` (server-reflexive) candidates. Fails behind
  symmetric NAT.
- **TURN** (RFC 5766): relay fallback. Client allocates a relay address on the
  TURN server; all media is forwarded through it. "Works when: always." Adds
  latency (+20–80 ms/direction typical) but guarantees connectivity behind
  symmetric NAT, strict firewalls, mobile carriers. [telnyx ICE&TURN; wowza
  STUN/TURN; signalwire]
- **ICE** (RFC 5245 family): orchestrates. Gathers host / srflx / relay
  candidates, connectivity-checks them in priority order (host > srflx > relay),
  picks the best working path. [coturn feature list references RFC 5245 etc.]

### When direct P2P suffices
- Same cluster, Cilium/CNI pod networking: **host candidates only, no STUN/TURN.**
  Both peers already know their routable pod IPs. STUN would only rediscover the
  pod IP (useless); TURN is pure overhead.

### When TURN is required
- Cross-cluster robot meshes (different L3 networks / NAT between clusters).
- Browser viewers behind NAT/firewall (the classic WebRTC case — most consumer
  calls use srflx or relay). Symmetric NAT, corporate firewalls → relay.
- Mobile/carrier networks that block P2P UDP.

### Minimal coturn deployment
- coturn is the standard FOSS TURN **and** STUN server (C, implements RFC 5389 /
  5766). Single binary, `apt install coturn` or
  `docker run -p 3478:3478 -p 3478:3478/udp -p 5349:5349 -p 5349:5349/udp
  -p 49152-65535:49152-65535/udp coturn/coturn`. For WebRTC use the TURN REST
  API (time-limited creds) and `turns:` (TLS, port 5349). [github.com/coturn/coturn]
- Minimal: one Deployment + Service (UDP/TCP 3478, TLS 5349, relay range
  49152–65535/UDP). Optional: shared secret auth (REST API) to avoid static
  passwords; Prometheus metrics available.

### Does coturn break the "zenoh-only signaling" spirit?
- **No.** TURN is a *media/data relay*, not a signaling channel. It never
  exchanges SDP or ICE — it only forwards RTP/RTCP once ICE has selected a relay
  candidate. Signaling still rides zenoh exclusively. Adding coturn adds a media
  plane component, not a signaling service. The "zenoh-only signaling infra"
  decision is about *how peers find each other and exchange offers/answers*,
  which is untouched. [ICE/TURN distinction per RFC 5245/5766; signalwire
  description of relay-only role]

---

## 3. ICE strategy the signaling should assume

- **Default (v1, same-cluster robot pods): host candidates only (UDP).**
  Simplest, lowest latency, no extra infra. ICE server list can be empty.
- **Future / cross-cluster / browser viewers: allow srflx + relay.**
  The `RTCPeerConnection` is constructed with an `iceServers` list
  (STUN/TURN URLs). This list is **deployment configuration**, not part of the
  signaling envelope.
- **Envelope implication (ticket 01):** carry SDP + candidates opaquely
  (Trickle ICE: exchange `RTCIceCandidate` strings as they are gathered). The
  envelope must NOT bake in host-vs-relay assumptions — it just forwards
  whatever ICE produces. webrtc-rs / the RTC stack generates the candidates;
  zenoh carries them verbatim. So:
  - envelope = `{ offer/answer SDP, candidate strings, mline index, sdpMid }`
  - NO need to add relay-specific fields — relay info lives in iceServers, which
    is out-of-band config.
  - This keeps ticket 01's schema transport-agnostic, as already intended.

---

## 4. Ferrous / no-unsafe check

- Our crate policy: no `unsafe`, no dependencies (per AGENTS.md). This ticket is
  infra/config only.
- webrtc-rs (the chosen P2P stack) performs all ICE/STUN/TURN internally; our
  code only serializes SDP + candidate strings and publishes them over zenoh.
- `grep -rn "unsafe"` across `src/` would show none related to ICE; and there is
  currently **no WebRTC client code in `src/`** (engine.rs is a rule engine, not
  RTC). So the connectivity decision adds zero `unsafe` surface to our Rust.
- Risk surface for `unsafe` would only appear if we ever hand-rolled raw socket
  ICE or FFI to a native TURN lib — explicitly out of scope here.

---

## Source list (primary)
- Kubernetes cluster networking model — kubernetes.io/docs/concepts/cluster-administration/networking/
- Cilium pod-to-pod connectivity — docs.cilium.io/en/stable/network/kubernetes/intro/
- Istio application requirements / sidecar interception — istio.io/latest/docs/ops/deployment/application-requirements/
- coturn (TURN+STUN server, RFC 5389/5766) — github.com/coturn/coturn
- ICE/STUN/TURN semantics — RFC 5245, RFC 5389, RFC 5766; telnyx "How ICE & TURN Work"; wowza STUN/TURN; signalwire STUN-vs-TURN-vs-ICE
