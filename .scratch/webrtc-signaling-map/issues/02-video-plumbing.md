# Ticket 02: Research v4l2 capture -> webrtc-rs video track plumbing

Label: `wayfinder:research`
Status: resolved
Blocked by:

## Question

Decide how in-container v4l2 camera capture becomes a WebRTC outbound video track,
under the hard ferrous / no-unsafe constraint (our code).

Resolve via a `/research` subagent. Investigate:
- The `v4l` crate (safe Rust, locked in client-container map): how to open
  `/dev/videoN`, enumerate formats, and pull frames (MMapStream / UserPtrStream).
- webrtc-rs outbound track API: `webrtc::track::track_local::track_local_static_sample`
  or similar; how to push encoded video frames as RTP. What codec/encoding the
  track expects (VP8/VP9/H.264) and whether we must encode raw v4l2 frames
  (typically YUYV/NV12) into that codec ourselves, or rely on a hardware encoder.
- The frame pipeline shape: v4l2 raw frame -> encode -> RTP sample -> track. Which
  crates do encoding without `unsafe` on our side (e.g. `openh264`/`rav1e` posture,
  or offload to a gstreamer/ffmpeg sidecar). Flag any crate that forces `unsafe`.
- Whether the signaling (ticket 01) needs to carry codec/SDP metadata — note for 01.

Capture findings on a throwaway `research/webrtc-video-plumbing` branch and post a
gist + branch/commit reference as the resolution comment.
