# Design: WebRTC Media Pipeline (class-3 video)

- **Date:** 2026-07-18
- **Status:** approved (design review)
- **Depends on:** `webrtc-signaling-map` (signaling schema, `signaling.rs` skeleton),
  `transport-protocol-map` (zenoh hybrid, class-3 = WebRTC), `client-container-arch-map`
  (DaemonSet, v4l device access).

## Goal

Make the `flo` client actually stream robot camera video peer-to-peer over WebRTC,
using **GStreamer under the hood for capture + hardware-accelerated encode**, and
**webrtc-rs** for the peer connection, ICE, DTLS, and RTP packetization. Signaling
rides the already-locked zenoh mesh (`signaling.rs`) unchanged.

The demo must let a user with two terminals (or a Jetson + a laptop) see live video
flow between two `cargo run` nodes, with no camera required to exercise the path
(`videotestsrc`).

## Locked decisions (from prior maps, reaffirmed)

- Class-3 video = WebRTC, P2P robot-to-robot. Browser viewer join = later map.
- Signaling = zenoh (no separate service). SDP + trickled ICE over
  `robot/<id>/signal/<peer>/{offer,answer,ice}`, presence on
  `robot/<id>/signal/presence`. Envelope `{sdp, kind, from, to, ice[]}`.
- No STUN/TURN for v1 (same-cluster pod networking gives direct host candidates).
  TURN is a future, out-of-band media relay — does not change signaling.
- `#![forbid(unsafe_code)]`. Safe-Rust crates only in *our* code. `openh264`
  (C FFI encoder) is **rejected**; encoding is GStreamer-native. Crate-internal
  FFI in dependency C libs (gstreamer, webrtc) is acceptable.

## Decisions made in this design (brainstorming, 2026-07-18)

1. **GStreamer role = encoder + capture only.** webrtc-rs owns the PeerConnection.
   This keeps `signaling.rs` and the locked signaling schema untouched.
2. **Handoff = `appsink` → webrtc-rs `TrackLocalStaticSample`.** GStreamer pulls
   encoded H.264 sample buffers into Rust; each is written to the webrtc-rs track.
   Chosen over `rtpbin`→localhost-UDP (which would re-own RTP packetization and add
   a fragile loopback hop) for simplicity and portability. The expensive NVENC
   encode stays hardware-accelerated and zero-copy *within* the GStreamer pipeline;
   only the final handoff copies the already-encoded buffer — negligible at robot scale.
3. **Default codec = H.264.** Encoder element auto-selected at runtime:
   `nvv4l2h264enc` on Jetson (NVENC, zero-copy NVMM path), `x264enc` on dev x86.
   One Rust code path; hardware where available, software fallback otherwise.
   AV1/`rav1e` and VP8 left as a later `--video-codec` toggle (not in v1).
4. **Scope = full P2P, two live nodes.** `cargo run --robot-id 7 --video-peer 8`
   and `cargo run --robot-id 8 --video-peer 7` exchange real video. Not a loopback
   slice, not flag-gated-only.

## Architecture & data flow

```
camera / videotestsrc
   │  GStreamer source: v4l2src on Jetson (--video-device), videotestsrc in demo
   ▼
GStreamer pipeline:
   src ! capsfilter(NV12, WxH, fps)
       ! nvv4l2h264enc            (Jetson NVENC)   OR   x264enc  (dev x86)
       ! h264parse ! appsink(name=enc, drop=true, max-buffers=2)
   ▼  (encoded H.264 sample buffers, pulled in Rust)
MediaPipeline (src/media.rs)
   │  appsink pull → Sample(encoded bytes + pts)
   ▼
webrtc-rs PeerConnection
   │  pc.add_track( TrackLocalStaticSample{ codec: H264 / 90000 } )
   │  track.write_sample(encoded bytes)   ← per appsink pull (tokio task)
   ▼
SDP offer/answer + trickled ICE
   │  rides EXISTING zenoh signaling (signaling.rs): publish_offer/answer/ice,
   │  run_signal_receiver, subscribe_presence — UNCHANGED, now driven
   ▼
remote peer (2nd cargo run node) receives; OnTrack asserts packet flow
```

## Module layout & interfaces

### `src/media.rs` (new) — `MediaPipeline`

- `Codec` enum: `H264` (v1; `Av1`, `Vp8` reserved).
- `MediaPipeline::build(source: &SourceSpec, codec: Codec, width, height, fps)`
  → builds the GStreamer pipeline (`gst::Pipeline`) with the encoder element chosen
  by `gst::ElementFactory::find("nvv4l2h264enc")` probe (Jetson) else `x264enc`.
  `SourceSpec`: `Videotest` (demo) | `V4l2(device_path)`.
- `MediaPipeline::start(&self, on_sample: impl Fn(Sample) + Send + 'static)`
  → spawns a tokio task pulling `appsink` buffers and invoking `on_sample` with the
  encoded bytes + presentation timestamp.
- `MediaPipeline::stop(&self)` → `pipeline.set_state(Null)`.
- Safe-Rust `gstreamer`, `gstreamer-app`, `gstreamer-video` crates. No `unsafe`.

### `src/video.rs` (new) — WebRTC glue

- `start_video(robot_id: &str, peer_id: &str, transport: &Transport) -> Result<()>`
  - builds a webrtc-rs `PeerConnection` (no ICE servers for v1),
  - creates the H.264 `TrackLocalStaticSample` (clock_rate 90000), `add_track`,
  - `on_ice_candidate` → `signaling::publish_ice`,
  - creates offer → `signaling::publish_offer`,
  - `on_track` → log `▶ video track received from <peer>` (render out of scope).
  - implements `signaling::SignalHandler`: `on_offer` (if we are answerer),
    `on_answer` (apply to PC), `on_ice` (add candidate to PC).
- Owns the `Arc<PeerConnection>`; panics are avoided — errors logged, node stays up.

### `src/main.rs` (changed)

- New flags:
  - `--video-peer <id>` — peer robot id to call (enables video mode).
  - `--video-device <path>` — e.g. `/dev/video0`; default = `videotestsrc` (demo).
  - `--video-codec <h264>` — default `h264`. v1 accepts only `h264`; passing
    `av1`/`vp8` returns a clear "unsupported in v1" error (enum reserves them).
  - `--video-self-test` — single-process headless encode assertion (see Testing).
- When `--video-peer` is set, `video::start_video` is launched (tokio task) alongside
  the rule engine and simulator.
- Existing `--robot-id` / `--config` / `--simulate*` unchanged.

### `Cargo.toml` (changed)

- Add `webrtc` (v0.17.x, mature tokio line — NOT the v0.20.0-rc Sans-I/O rewrite),
  `gstreamer`, `gstreamer-app`, `gstreamer-video` (current release). `openh264` NOT added.
- Keep `forbid(unsafe_code)`. Document in `GETTING_STARTED.md` that system GStreamer
  (>= 1.14, with `x264enc`/`h264parse`/`videotestsrc`, plus `nvv4l2h264enc` on Jetson)
  must be installed (apt: `gstreamer1.0-plugins-{base,good,bad,ugly}`, `gstreamer1.0-libav`).

## Demo flow (no camera needed to exercise)

1. Terminal 1: `cargo run --robot-id 7 --video-peer 8`
2. Terminal 2: `cargo run --robot-id 8 --video-peer 7`
3. Node 7 captures via `videotestsrc` (or `v4l2src` if `--video-device` given),
   GStreamer encodes H.264, webrtc-rs PC offers to 8 over zenoh signaling.
4. Node 8 answers; ICE trickles; video flows 7→8. Node 8 logs `▶ video track received`.
5. If `nvv4l2h264enc` absent (dev laptop), auto-fallback to `x264enc` with an INFO line.

## Error handling

- GStreamer pipeline build failure → `error!` + node stays up (rule engine unaffected);
  video is best-effort.
- Peer offline / never answers → offer published, PC times out gracefully; no panic.
- `write_sample` backpressure → `appsink` `max-buffers=2, drop=true` drops frames
  rather than buffering infinitely (correct for live video).

## Testing (no GPU / no camera in CI)

- `cargo clippy` + `cargo build` must pass with the new deps (our code `forbid(unsafe_code)`).
- `--video-self-test` mode: single process builds the GStreamer pipeline against
  `videotestsrc`, pulls N appsink samples, asserts non-empty encoded bytes and a valid
  Annex-B start code (`00 00 00 01`). Proves encode works headless. Run manually
  (env lacks GStreamer; not in `cargo test`).
- Document the 2-terminal recipe in `GETTING_STARTED.md`.

## Out of scope (later maps)

- Receiver-side decode/render (display the video).
- Browser viewer join path (signaling is peer-agnostic; offer origin differs).
- TURN / cross-cluster relay (out-of-band `iceServers` config).
- Multi-track / SFU.
- AV1 (`rav1e`/`av1enc`) and VP8 codec toggles.
- DaemonSet camera device wiring + permissions at deploy time.

## Risks

- GStreamer system dependency must be present at runtime (documented; self-test helps).
- webrtc-rs v0.17.x API surface for `TrackLocalStaticSample` + ICE candidates — verify
  against the pinned version during implementation; adjust `video.rs` accordingly.
- `appsink` sample timing/pts must be monotonic for smooth RTP; clamp/handle wraps.
