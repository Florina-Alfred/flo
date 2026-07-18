# Ticket 03: Research k8s P2P WebRTC connectivity (STUN/TURN vs mesh)

Label: `wayfinder:research`
Status: open
Blocked by:

## Question

Decide the connectivity assumptions the WebRTC signaling (ticket 01) can rely on
for pod-to-pod P2P under Kubernetes, under the ferrous / no-unsafe constraint.

Resolve via a `/research` subagent. Investigate:
- Pod-to-pod reachability in CNI meshes relevant to this repo (Cilium, Istio): do
  P2P ICE candidates (host/UDP) traverse directly, or is a STUN/TURN server needed?
- STUN vs TURN: when direct P2P suffices (same cluster, Cilium pod networking) vs
  when TURN relay is required (cross-cluster, NAT, browser viewers). What a minimal
  STUN/TURN deployment looks like (coturn) and whether it breaks the "zenoh-only
  infra" spirit of the signaling decision (it's media relay, not signaling).
- ICE strategy the signaling should assume: host candidates only (cluster-internal)
  vs srflx/relay. This shapes what ticket 01's envelope must carry.
- Confirm none of this touches `unsafe` in our Rust code (it's infra/config, not code).

Capture findings on a throwaway `research/webrtc-connectivity` branch and post a
gist + branch/commit reference as the resolution comment. This ticket informs but
does NOT block ticket 01 (signaling schema is transport-agnostic).

## Resolution

**Recommended ICE strategy: host candidates only (UDP) for the v1 same-cluster
pod-to-pod path.** No STUN/TURN needed when both peers are pods in one cluster —
the k8s network model gives every pod a routable IP, so ICE host↔host
connectivity checks succeed directly on both Cilium (eBPF, no pod NAT/proxy) and
Istio (Envoy intercepts by Service IP; a pod-IP UDP media stream passes through
at L3/L4, bypassing mTLS — which is what we want for RTP).

**STUN/TURN component: NOT in the v1 scope; pluggable for later.** A TURN relay
(coturn) is only required for cross-cluster meshes (different L3/NAT) and for
future browser viewers behind NAT/firewall. coturn is FOSS (RFC 5389 STUN +
RFC 5766 TURN), one Deployment + Service (UDP/TCP 3478, TLS 5349, relay
49152–65535/UDP), REST-API auth. Critically, **TURN is a media relay, not a
signaling service** — adding coturn does not break the zenoh-only signaling
decision; signaling still rides zenoh exclusively. Keep the `iceServers` list as
deployment config, not in the envelope.

**Signaling envelope must carry:** SDP + ICE candidates opaquely via Trickle ICE
(`{ offer/answer, candidate strings, mline index, sdpMid }`). It must stay
transport-agnostic and NOT bake in host-vs-relay — relay info lives in iceServers
(out-of-band). This matches ticket 01's existing design.

**Ferrous check:** confirmed none of this touches `unsafe` in our Rust. webrtc-rs
handles all ICE/STUN/TURN; our code only serializes SDP + candidate strings over
zenoh. No WebRTC client code exists in `src/` yet — this is pure infra/config.

**Branch/commit:** `research/webrtc-connectivity` @ `bc8b77e`
(<https://github.com/.../flo/commit/bc8b77e>). Full notes:
`.scratch/webrtc-signaling-map/research/03-connectivity-findings.md`.

**Sources (primary):** kubernetes.io cluster networking model;
docs.cilium.io/.../kubernetes/intro (pod-to-pod); istio.io application-requirements
(sidecar interception); github.com/coturn/coturn; RFC 5245/5389/5766; telnyx/wowza/
signalwire ICE-TURN explainers.
