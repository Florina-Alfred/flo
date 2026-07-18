# Resolution: Ticket 02 — v4l2 -> webrtc-rs video plumbing

**Status of ticket:** unchanged (open). This file is a resolution note, not the ticket.

## Gist

Resolved how in-container v4l2 camera capture becomes a WebRTC outbound video
track under the ferrous / no-unsafe constraint. Findings committed on branch
`research/webrtc-video-plumbing`, commit `6cb5564`.

Full notes: `.scratch/webrtc-signaling-map/research/02-video-plumbing-findings.md`
(branch `research/webrtc-video-plumbing`, commit `6cb55646f2e6e8616b27dfa7b7f6b415c2491237`).

## Key recommendation

- **Capture:** `v4l` 0.14.0 `MMapStream` over `/dev/videoN`, raw `YUYV`/`NV12`
  (or `H264` if the UVC cam offers it). `v4l` is safe Rust — no `unsafe` in our code.
- **Encode (unsafe-free):** prefer a **gstreamer/ffmpeg sidecar** that reads the
  raw v4l2 stream and emits AV1 (or H.264) bytes our process feeds to the track;
  pure-safe-Rust fallback is **`rav1e`** (AV1). **Reject `openh264`** (FFI over
  Cisco C lib via `openh264-sys2` — taints the tree with `unsafe`). MJPEG
  passthrough is rejected: webrtc-rs has no MJPEG payload type, so it must still
  be decoded+re-encoded.
- **Track API:** `webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample::new(RTCRtpCodecCapability{ mime_type, .. }, id, stream_id)` then `track.write_sample(&media::Sample{ data, duration, .. }).await`. The track packetizes encoded `data` to RTP internally; it does NOT encode. Supported video MIME types include `MIME_TYPE_AV1`, `MIME_TYPE_H264`, `MIME_TYPE_VP8/VP9`.
- **Signaling dependency (ticket 01):** the signaling channel MUST relay SDP /
  `RTCRtpCodecCapability` (mime_type, clock_rate, sdp_fmtp_line) for offer/answer
  negotiation. Codec must be registered in `MediaEngine` before `RTCPeerConnection`
  creation. Flagged as a hard dependency on ticket 01.
- **Ferrous confirmation:** `v4l`, `webrtc` 0.17.1, `rav1e`, and the sidecar
  approach keep OUR code unsafe-free. Only `openh264` forces `unsafe` (FFI).
