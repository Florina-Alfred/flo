# Research: v4l2 capture -> webrtc-rs outbound video track

Branch: `research/webrtc-video-plumbing`
Verified against primary sources (docs.rs, webrtc-rs examples, codec crate docs) on 2026-07-18.

## 1. The `v4l` crate (v4l 0.14.0, safe bindings to v4l2)

- Crate is explicitly a **safe** Rust wrapper (`pub use ... as v4l_sys` over `v4l2-sys-mit`).
  No `unsafe` is exposed in the public API we use.
- Open a device by index or path:
  `v4l::Device::with_path("/dev/video0")` or `Device::new(0)`.
- Enumerate formats / frame sizes via `dev.enum_formats()`, `dev.enum_framesizes(fourcc)`.
  Query/set the current format with `v4l::video::Capture::get_format` / `set_format`
  using `v4l::Format { fourcc, width, height, .. }` where `fourcc` is a `v4l::FourCC`.
- Pull frames with a streaming IO object (from `v4l::io::traits::CaptureStream`):
  - `v4l::io::mmap::MMapStream::with_buffers(&mut dev, Type::VideoCapture, 4)`
    returns `Result<(buf, meta)>` per `stream.next()`. Zero-copy; read-only.
  - `v4l::io::userptr::UserptrStream` for host-allocated buffers (also zero-copy).
- Typical raw pixel formats v4l2 cameras expose (FourCC): `YUYV` (YUY2),
  `NV12` (semi-planar YUV420), `MJPEG`, `RGB3`, `H264` (some UVC cams output
  already-encoded H.264). `YUYV`/`NV12` are uncompressed and must be encoded
  before WebRTC. **`MJPEG` capture does NOT avoid encoding** for WebRTC — see §3.

## 2. webrtc-rs outbound track API (webrtc 0.17.1)

- Local track lives at `webrtc::track::track_local`:
  - `track_local_static_sample::TrackLocalStaticSample` — pre-set codec, push
    **Samples** (encoded frame payloads; the track packetizes to RTP internally).
  - `track_local_static_rtp::TrackLocalStaticRTP` — push raw RTP packets yourself.
- Construction:
  `TrackLocalStaticSample::new(RTCRtpCodecCapability { mime_type, clock_rate, .. }, id, stream_id)`
  then `track.write_sample(&media::Sample { data, duration, .. }).await`.
  (`write_sample_with_extensions` / `sample_writer()` builder also available.)
- The track does **not** encode. It expects the `mime_type` to name an
  RTP-supported codec, and `data` to be already-encoded codec bytes (one or more
  samples). Supported video MIME types in `webrtc::api::media_engine`:
  `MIME_TYPE_VP8`, `MIME_TYPE_VP9`, `MIME_TYPE_H264`, `MIME_TYPE_HEVC`,
  `MIME_TYPE_AV1`. **There is NO MJPEG video MIME type** — MJPEG cannot be sent
  directly; the encoder must produce VP8/VP9/H.264/HEVC/AV1.
- `RTCRtpCodecCapability` carries SDP-relevant fields (clock_rate, channels,
  sdp_fmtp_line) — this is what the track advertises in the SDP offer/answer.

## 3. Encode pipeline (raw v4l2 frame -> encoded -> Sample -> track)

Options evaluated against the no-unsafe-in-our-code constraint:

- **(a) Capture MJPEG, wrap without re-encode** — REJECTED. webrtc-rs has no
  MJPEG payload type. MJPEG must be decoded to raw then re-encoded to a supported
  codec. No saving; adds a decode step.
- **(b) Software encoder in safe Rust:**
  - `rav1e` 0.8 (AV1) — self-described "the fastest and safest AV1 encoder";
    pure-Rust with optional SIMD; its `Context`/`Frame`/`Packet` API is safe.
    **Keeps our code unsafe-free.** AV1 is a first-class `MIME_TYPE_AV1` in
    webrtc-rs. Caveat: AV1 software encoding is CPU-heavy at high res/fps;
    acceptable for modest robot-camera resolutions.
  - `openh264` 0.9 (H.264) — wraps Cisco's C library via `openh264-sys2`
    (FFI, `cc`-compiled). Its own FAQ states it gives **no** Rust safety
    guarantees for the video handling. Acceptable only if we accept a C dependency
    + FFI in the tree; **violates the no-unsafe posture on principled grounds**
    (transitive `unsafe` through FFI). Flagged.
- **(c) Offload encoding to a sidecar (gstreamer/ffmpeg)** — the encoder runs as
  a separate process/container; our Rust code only reads encoded bytes (e.g. an
  IVF/OBU or Annex-B H.264 pipe) and feeds them to `write_sample`. **Fully
  unsafe-free in our code**; gives hardware-accel (VAAPI/NVENC) and H.264/VP8/VP9
  flexibility. Cost: sidecar lifecycle/ops + IPC.

**Recommendation:** primary path = **(c) sidecar encoder** when hardware/CPU
budget matters; **(b) rav1e** as the in-process safe-Rust fallback. Avoid
openh264. Do not pursue MJPEG passthrough.

## 4. Signaling dependency (ticket 01)

- YES — the signaling protocol must carry codec/SDP metadata. `RTCRtpCodecCapability`
  (mime_type, clock_rate, sdp_fmtp_line) is what `RTCPeerConnection` puts in the
  SDP offer/answer. The negotiation (offer/answer exchange) must be transported by
  the signaling channel decided in ticket 01. The chosen codec (AV1 via rav1e, or
  H.264/VP8 via sidecar) must be registered in the `MediaEngine` before creating
  the PeerConnection, and the remote must support it — so signaling needs to
  expose/relay the SDP. **Note this as a hard dependency for ticket 01.**

## 5. Ferrous / no-unsafe confirmation

- `v4l` 0.14.0: safe public API; no `unsafe` required in our code.
- `webrtc` 0.17.1: safe API (`TrackLocalStaticSample`, `RTCPeerConnection`,
  `MediaEngine`); no `unsafe` in our usage.
- `rav1e` 0.8: safe encoder API; **no-unsafe-friendly** (preferred).
- `openh264` 0.9: FFI over C via `openh264-sys2`; **forces `unsafe`-tainted
  dependency** — excluded under the ferrous constraint.
- Sidecar (gstreamer/ffmpeg): external binary; **trivially keeps our code
  unsafe-free.**

## Summary recommendation

Capture raw `YUYV`/`NV12` (or `H264` if the UVC cam offers it) via `v4l`
`MMapStream`; encode out-of-process with a gstreamer/ffmpeg sidecar into AV1 (or
H.264), reading encoded bytes back into `TrackLocalStaticSample` via
`write_sample`; fall back to `rav1e` in-process for a pure-safe-Rust path.
Signaling (ticket 01) must relay SDP/codec capability.
