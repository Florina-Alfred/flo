# Map: WebRTC Signaling for Class-3 Video

Label: `wayfinder:map`

## Destination

Lock the WebRTC signaling design for class-3 (live camera video) so the platform
can stream robot camera feeds peer-to-peer, with future browser-UI viewing.

Decided at chart time (pinned via grilling):
- **Zenoh-backed signaling** — SDP/ICE exchange rides the already-locked Zenoh
  hybrid mesh. No separate signaling service; consistent with the transport map.
- **P2P robot-to-robot** — webrtc-rs peer connections between robots; browsers
  join the same mesh later for UI viewing (the original reason class 3 was WebRTC).
- **Key-expr per peer** for signaling:
  `robot/<id>/signal/<peer>/{offer,answer,ice}` carrying JSON SDP/ICE.
- **Offerer-publishes** — the camera owner (offerer) publishes an offer on its own
  signal key-expr; the answerer subscribes, then publishes its answer on the
  answerer's signal key-expr. ICE candidates trickle over the same channel.
- **In-container v4l2 capture** — the camera is captured locally via the `v4l`
  crate (safe Rust, locked in the client-container map) and pushed as a WebRTC
  video track. Local capture, remote publish.

## Notes

- Domain: robotics orchestration over Kubernetes; class-3 video on top of the
  locked transport (zenoh hybrid, `robot/<id>/local/**`, `fleet/**` namespaces).
- Skills every session should consult: `/grilling`, `/domain-modeling`, `/research`.
- Standing preferences: ferrous / no-unsafe is non-negotiable (carried from the
  transport map). Reuse the zenoh session; do NOT spin up a second transport.
- Depends on: transport-protocol-map (zenoh crate, hybrid topology, namespaces)
  and client-container-arch-map (DaemonSet, v4l device access, `flo` session).
- Issue tracker: local markdown. `Status:` = open/claimed/resolved;
  `Blocked by:` lists ticket numbers that must resolve first.

## Decisions so far

<!-- index — one line per resolved ticket, gist + link to the ticket holding detail -->

- **02 (Video plumbing):** Capture via `v4l` MMapStream (safe Rust). webrtc-rs
  `TrackLocalStaticSample` takes already-encoded bytes (it RTP-packetizes, doesn't
  encode); MIME AV1/H264/VP8/VP9/HEVC. Keep our code unsafe-free via a
  gstreamer/ffmpeg **sidecar** encoder (or pure-Rust `rav1e` fallback); **reject
  `openh264`** (C FFI). Signaling must carry SDP + `RTCRtpCodecCapability`
  (mime, clock_rate, fmtp). See `issues/02-video-plumbing-findings.md`.
- **03 (Connectivity):** Same-cluster pod networking (Cilium/Istio) gives direct
  host-candidate P2P — **no STUN/TURN for v1**. TURN (coturn) only for
  cross-cluster/browser viewers; it's a media relay, not signaling, so it does NOT
  violate the zenoh-only signaling decision. Envelope carries SDP + trickled ICE
  candidate strings opaquely; `iceServers` stays out-of-band config. See
  `issues/03-connectivity-findings.md`.
- **01 (Signal schema + flow):** **Presence key-expr** `robot/<id>/signal/presence`
  for peer discovery (each client publishes its id + offered camera streams).
  **Minimal JSON envelope** `{sdp, kind: offer|answer, from, to, ice:[candidate]}`
  with ICE trickle appending candidates. **Offerer-publishes**: offerer puts offer
  on `robot/<offerer>/signal/<answerer>/offer`; answerer replies answer on
  `robot/<answerer>/signal/<offerer>/answer`; ICE on respective `…/ice`. Reuses the
  `flo` zenoh session; safe Rust only. See `issues/01-signal-schema.md`.
- **04 (Signaling skeleton):** `src/signaling.rs` implemented — key-expr builders
  (`SIGNAL_PRESENCE/OFFER/ANSWER/ICE_KEY`), `serde` `SignalMessage`/`IceCandidate`/
  `Presence` structs, `publish_presence` + `run_signal_receiver` (offers/answers/ice
  dispatched to a `SignalHandler`) + `subscribe_presence`. `Transport::publish_json`
  added for best-effort signaling puts. Wired into `main.rs` (presence publish +
  logging handler); publisher fns reserved for the future media module. `#![forbid(unsafe_code)]`.

## Not yet specified

- **Track plumbing + media pipeline**: how v4l2 frames become RTP/video frames on a
  webrtc-rs track (encoding via gstreamer/ffmpeg sidecar or `rav1e`; codec/SDP
  metadata from ticket 02) — a future implementation map; this map only locks
  signaling. The `SignalHandler` in `main.rs` is a logging placeholder until then.
- **Browser join path**: how a future browser viewer receives the same stream
  (signaling is peer-agnostic, but the offer origin differs) — flagged for a later
  map; this map locks robot-to-robot only.
- **v4l device name / permissions** under the DaemonSet (locked in client-container
  map) — operational detail, filled at deploy time.
- **STUN/TURN for cross-cluster/browser** (ticket 03): out-of-band `iceServers`
  config; not part of the envelope. Deploy when cross-cluster or browser viewers ship.

## Out of scope

<!-- work ruled beyond the destination; closed, never graduates -->

- K8s platform shape (Cilium vs Istio, STUN/TURN server deployment) — separate map.
- Browser/video UI implementation — separate future map.
- The transport/protocol decision and client-container architecture — already
  locked in their own maps.
- These belong to other efforts, not this WebRTC-signaling lock.
