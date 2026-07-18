# WebRTC Rust crate research — ferrous / no-unsafe + live video fitness

Throwaway research branch for Wayfinder ticket 02.
Context: robot orchestration platform (k8s, Rust, ferrous/no-unsafe).
Locked decision: WebRTC carries QoS class 3 (live camera video), chosen for
FUTURE browser-UI support. Constraint: NO `unsafe` anywhere in OUR code.

## 1. Candidate crates & ferrous / safety posture

### A. `webrtc` / `rtc` (webrtc-rs)  — RECOMMENDED
- Repo: https://github.com/webrtc-rs/webrtc  (crates.io: `webrtc`)
  Sans-I/O core: https://github.com/webrtc-rs/rtc  (crates.io: `rtc`)
- Pure Rust implementation of the WebRTC stack (ICE, DTLS, SRTP/SRTCP, SCTP,
  RTP/RTCP, SDP, Data Channels, Media Tracks), originally a Rust rewrite of the
  Pion (Go) stack. No C/C++ dependencies.
- The core `rtc` crate is explicitly "pure safe Rust implementation"
  (webrtc.rs blog, announcing rtc 0.3.0, 2026-01-04). No FFI, no cxx, no libwebrtc.
- Architecture split:
  - `rtc` (v0.3.x): Sans-I/O protocol core — you drive the I/O loop, protocol
    logic is pure/testable, runtime-agnostic.
  - `webrtc` (v0.17.x mature / v0.20.x-rc runtime-agnostic rewrite over `rtc`):
    async `PeerConnection` API with a `Runtime` trait (default `runtime-tokio`,
    `runtime-smol` available).
- Version reality (crates.io, 2026-07):
  - `webrtc` 0.17.1 = final feature release of the Tokio-coupled line;
    bug-fix-only maintenance. Mature, 5M+ downloads, 81 reverse deps.
  - `webrtc` 0.20.0-rc.x = new Sans-I/O / runtime-agnostic rewrite (RC, July 2026).
    API expected stable but still pre-1.0; low download count.
- **Our-side `unsafe` posture: NONE REQUIRED.** webrtc-rs is safe Rust end to
  end. ferrous will compile it; we add no `unsafe` in our call sites.

### B. `livekit` (LiveKit Rust client/server SDK) — REJECTED for this constraint
- Repo: https://github.com/livekit/rust-sdks  (crates.io: `livekit` 0.7.45)
- It is a client SDK that talks to a LiveKit SFU server. It wraps Google's
  libwebrtc via `libwebrtc` / `webrtc-sys` (crates.io: `webrtc-sys` — "Unsafe
  bindings to libwebrtc", built with cxx.rs over C++).
- Implication: even though our own `.rs` call sites may be `safe`, the transitive
  dependency chain pulls in `unsafe` FFI to C++. That contradicts the hard
  "no unsafe anywhere in our code" / ferrous posture. Rejected.

### C. Other notes
- No serious pure-Rust alternative to webrtc-rs exists for a full WebRTC stack.
  (e.g. `str0m` is another pure-Rust WebRTC media crate worth a future look, but
  webrtc-rs is the mainstream, most-documented choice and is the natural fit.)

## 2. Fitness for live camera video (class 3)

- WebRTC is purpose-built for real-time media: sub-second latency, built-in
  congestion control (TWCC/REMB), NACK/RTCP feedback, and SRTP encryption.
  Ideal match for live camera video.
- Topology: webrtc-rs provides P2P peer connections (mesh). For robot-to-robot
  direct streams this is fine. For fan-out to many robots or to a future
  browser-UI audience, a SFU is the standard scale-out — but the webrtc-rs crate
  itself is peer/endpoint logic, NOT an SFU. The webrtc.rs org also publishes an
  SFU (separate project) if needed later.
- Bandwidth: WebRTC adapts bitrate to available path; with simulcast/SVC (supported
  by `rtc`) a future SFU or heterogenous receivers (robots vs browser) can be
  served at different qualities from one source.
- Future browser-UI support: WebRTC is the browser-native real-time API — picking
  it means the future browser clients need zero extra protocol work. This is the
  original reason class 3 was locked to WebRTC.

## 3. Signaling prerequisites (FLAGGED — future dependency, NOT resolved)

- WebRTC media requires an out-of-band signaling channel to exchange SDP
  offer/answer and ICE candidates before a peer connection can form. webrtc-rs
  provides the SDP/ICE machinery but NOT the transport that moves SDP between
  peers.
- Under k8s this is an open design question: how do two robot pods (and later a
  browser client) discover each other and exchange SDP? Options: a dedicated
  signaling service (WebSocket/gRPC), reuse of the Zenoh control plane, or a
  sidecar. This is explicitly OUT OF SCOPE for the transport lock and is flagged
  as a known future-dependency for a later wayfinder map. Not resolved here.

## Recommendation
Adopt **webrtc-rs** (`webrtc` 0.17.1 for a stable Tokio-based start, or track
`0.20.0-rc` for the runtime-agnostic/Sans-I/O design). It is pure safe Rust —
no `unsafe` required on our side, ferrous-compatible. Pair with a separately
designed signaling channel (future ticket). Do NOT use LiveKit (pulls in unsafe
libwebrtc FFI).
