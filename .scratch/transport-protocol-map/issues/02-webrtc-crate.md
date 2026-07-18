# Ticket 02: Research WebRTC Rust crate compatible with ferrous/no-unsafe

Label: `wayfinder:research`
Status: resolved
Blocked by:

## Question

Decide which WebRTC Rust crate carries class 3 (live camera video), under the
hard ferrous / no-unsafe constraint, with future browser-UI support in mind.

Resolve via a `/research` subagent. Investigate:
- Available Rust WebRTC crates (e.g. `webrtc`/`webrtc-rs`, `livekit`, others) and
  their ferrous/safety posture — whether our usage requires `unsafe` on our side.
- Fitness for live camera video streams (latency, SFU vs P2P, bandwidth) in a
  robot-to-robot + future browser-UI context.
- Signaling prerequisites (how peers exchange SDP under k8s) — flagged here as a
  known future-dependency, not resolved in this ticket.

Capture findings on a throwaway `research/webrtc-transport` branch and post a
context pointer (gist + branch link) as the resolution comment.
