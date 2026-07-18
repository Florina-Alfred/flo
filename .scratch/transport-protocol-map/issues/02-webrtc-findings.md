# Resolution: 02 — WebRTC Rust crate compatible with ferrous/no-unsafe

(Research summary for Wayfinder ticket 02. The originating ticket is
`issues/02-webrtc-crate.md`; this file holds the resolution pointer only and
does not modify that ticket's `Status:` line.)

## Gist

Adopt **webrtc-rs** (`webrtc` crate, Sans-I/O core `rtc`) for QoS class 3
(live camera video). It is a **pure safe-Rust** WebRTC stack (ICE/DTLS/SRTP/
SCTP/RTP/SDP/DataChannels/MediaTracks) with no C/C++ dependencies, so **no
`unsafe` is required in our code** and it compiles under ferrous.

`livekit` was rejected: it wraps Google's libwebrtc via `webrtc-sys`
("Unsafe bindings to libwebrtc", cxx.rs over C++) — that transitive `unsafe`
FFI violates the hard no-unsafe constraint.

Fitness: WebRTC is purpose-built for real-time media (sub-second latency, SRTP,
TWCC/NACK congestion control, simulcast/SVC for heterogenous receivers), and is
the browser-native real-time API — so future browser-UI support needs no extra
protocol work. webrtc-rs gives P2P peer connections; an SFU for fan-out is a
separate concern (webrtc.rs publishes one) if robot→many or robot→browser
scale-out is needed later.

Signaling (SDP/ICE exchange under k8s) is **flagged as a known future-dependency**
— webrtc-rs supplies the SDP/ICE machinery but not the transport that moves SDP
between peers. Out of scope for the transport lock; deferred to a later map.

## Branch / commit reference

- Branch: `research/webrtc-transport` (throwaway)
- Commit: `f6bc3a19b2796b3a97f58125304749147ce550dc`
- Notes file: `RESEARCH-webrtc-transport.md` on that branch

## Key recommendation

Use `webrtc` 0.17.1 for a stable Tokio-based start (mature, bug-fix-maintained),
or track `0.20.0-rc` for the runtime-agnostic / Sans-I/O design. Pair with a
separately-designed signaling channel (future ticket). Do NOT use LiveKit.
