# WebRTC Media Pipeline (class-3 video) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream robot camera (or synthetic) video peer-to-peer over WebRTC, with GStreamer doing hardware-accelerated encode and webrtc-rs owning the PeerConnection, signaling over the existing zenoh mesh.

**Architecture:** GStreamer captures + encodes (videotestsrc/v4l2src → nvv4l2h264enc on Jetson, x264enc on dev) and pushes encoded H.264 samples out via `appsink`. A tokio task forwards each sample into a webrtc-rs `TrackLocalStaticSample`. The SDP offer/answer + trickled ICE ride the already-locked `signaling.rs` zenoh schema unchanged. GStreamer is feature-gated (`media` feature) because it needs system GStreamer libs; webrtc-rs is always built.

**Tech Stack:** Rust 2024, `webrtc` 0.17.x (mature tokio line), `gstreamer`/`gstreamer-app`/`gstreamer-video` 0.25.x (feature `media`), `bytes`, `zenoh` (existing), `serde`/`tokio` (existing).

## Global Constraints

- `#![forbid(unsafe_code)]` — our code stays unsafe-free. Crate-internal FFI in `webrtc`/`gstreamer` C deps is acceptable.
- `openh264` is **rejected** — encoding is GStreamer-native only. Do NOT add it.
- Class-3 = WebRTC, P2P robot-to-robot. Signaling = zenoh (no separate service). Schema from `signaling.rs` is reused **unchanged**.
- No STUN/TURN for v1 (same-cluster direct host candidates). TURN is future out-of-band.
- Default codec = H.264. Encoder auto-selected: `nvv4l2h264enc` on Jetson, else `x264enc`.
- `--video-codec` accepts only `h264` in v1 (av1/vp8 reserved, error if passed).
- GStreamer modules are feature-gated behind `media`; default `cargo build`/`cargo clippy` must stay green WITHOUT system GStreamer installed.
- CLI contract (unchanged parts): `cargo run` = sensor demo; `--robot-id`, `--config`, `--simulate*`.
- New flags: `--video-peer <id>`, `--video-device <path>`, `--video-codec <h264>`, `--video-self-test`.

---

## File Structure

- `Cargo.toml` — add `webrtc` (0.17), `gstreamer`/`gstreamer-app`/`gstreamer-video` (0.25, optional via `media` feature), `bytes`. Add `[features] media = ["dep:gstreamer", ...]`.
- `src/main.rs` — add `VideoArgs` parsing; call `video::start_video` when `--video-peer` set; `--video-self-test` path; gstreamer init when media feature on.
- `src/video.rs` (new, always built) — webrtc-rs PeerConnection glue + `SignalHandler` impl. Depends on `signaling` + `transport`.
- `src/media.rs` (new, `#[cfg(feature = "media")]`) — `MediaPipeline`: GStreamer pipeline build, encoder auto-select, `appsink` → sample callback.
- `src/codec.rs` (new, always built) — `Codec` enum + `h264_codec_capability()` pure helper (unit-testable, no GStreamer).
- `GETTING_STARTED.md` — 2-terminal video recipe + GStreamer install note.

---

## Task 1: Dependencies + CLI argument parsing

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs` (arg parsing only; no video wiring yet)
- Test: `src/main.rs` (unit test for arg parse) — see Step 1.

**Interfaces:**
- Produces: `VideoArgs { peer: Option<String>, device: Option<String>, codec: Codec, self_test: bool }` and `parse_video_args(iter) -> VideoArgs` (or integrated into existing `Args`).
- Produces: `Codec` enum (re-exported from `src/codec.rs`).

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_args() {
        let args = Args::parse_from(vec![
            "flo",
            "--robot-id", "7",
            "--video-peer", "8",
            "--video-device", "/dev/video0",
            "--video-codec", "h264",
        ]);
        assert_eq!(args.robot_id, "7");
        assert_eq!(args.video.peer.as_deref(), Some("8"));
        assert_eq!(args.video.device.as_deref(), Some("/dev/video0"));
        assert_eq!(args.video.codec, crate::codec::Codec::H264);
    }

    #[test]
    fn rejects_unknown_codec() {
        let r = std::panic::catch_unwind(|| {
            Args::parse_from(vec!["flo", "--video-codec", "vp8"]);
        });
        assert!(r.is_err());
    }
}
```

(Note: `Args::parse_from` is the existing hand-rolled parser; extend it. If the existing parser is a manual loop, add video keys there and keep tests passing by calling the same entry fn.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo parses_video_args 2>&1 | tail -20`
Expected: FAIL — `video` field / `Codec` does not exist yet.

- [ ] **Step 3: Add dependencies to Cargo.toml**

Append:

```toml
[dependencies]
webrtc = "0.17"
bytes = "1"

[dependencies.gstreamer]
version = "0.25"
optional = true

[dependencies.gstreamer-app]
version = "0.25"
optional = true

[dependencies.gstreamer-video]
version = "0.25"
optional = true

[features]
default = []
media = ["dep:gstreamer", "dep:gstreamer-app", "dep:gstreamer-video"]
```

Keep existing `zenoh`, `serde`, `tokio`, `serde_json`, `tracing`, `axum` deps untouched.

- [ ] **Step 4: Create `src/codec.rs`**

```rust
//! Video codec selection. Pure, no GStreamer — unit-testable.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// H.264 — default. Hardware (nvv4l2h264enc) on Jetson, x264enc on dev.
    H264,
    // Av1, Vp8 reserved for a later release.
}

impl Default for Codec {
    fn default() -> Self {
        Codec::H264
    }
}

impl std::str::FromStr for Codec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "h264" => Ok(Codec::H264),
            other => Err(format!("unsupported --video-codec '{other}' (v1 supports: h264)")),
        }
    }
}

/// Build the webrtc-rs codec capability for H.264 (clock rate 90 kHz, per RFC 6184).
pub fn h264_codec_capability() -> webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
    webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
        mime_type: webrtc::api::media_engine::MIME_TYPE_H264.to_owned(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1".to_owned(),
        rtcp_feedback: vec![],
    }
}
```

- [ ] **Step 5: Extend `Args` in `src/main.rs`**

Add the `video` field and parsing. Insert near the top struct:

```rust
mod codec;
use codec::Codec;

struct VideoArgs {
    peer: Option<String>,
    device: Option<String>,
    codec: Codec,
    self_test: bool,
}

struct Args {
    robot_id: String,
    config: Option<String>,
    simulate: bool,
    simulate_period_ms: u64,
    video: VideoArgs,
}
```

In the arg loop, handle:

```rust
"--video-peer" => { args.video.peer = iter.next().map(|s| s.to_string()); }
"--video-device" => { args.video.device = iter.next().map(|s| s.to_string()); }
"--video-codec" => {
    let v = iter.next().unwrap_or("h264");
    args.video.codec = v.parse().unwrap_or_else(|e| panic!("--video-codec: {e}"));
}
"--video-self-test" => { args.video.self_test = true; }
```

Ensure default `Args` sets `video: VideoArgs { peer: None, device: None, codec: Codec::H264, self_test: false }`.

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --bin flo 2>&1 | tail -20`
Expected: PASS (both `parses_video_args` and `rejects_unknown_codec`).

- [ ] **Step 7: Verify default build is clean (no GStreamer needed)**

Run: `cargo clippy --all-targets 2>&1 | grep -E "^error|^warning" | head`
Expected: empty (clean). `webrtc` compiles; gstreamer deps are optional and off by default.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/codec.rs
git commit -m "feat: add webrtc + video CLI args and Codec helper (media feature-gated)"
```

---

## Task 2: `src/media.rs` — GStreamer pipeline + appsink

**Files:**
- Create: `src/media.rs` (`#[cfg(feature = "media")]`)
- Test: `src/media.rs` — pure helper `select_encoder_element` unit test (no GStreamer init required).

**Interfaces:**
- Consumes: `crate::codec::Codec`, `crate::codec::h264_codec_capability` (Task 1).
- Produces:
  - `pub enum SourceSpec { Videotest, V4l2(String) }`
  - `pub struct MediaPipeline { ... }`
  - `impl MediaPipeline { pub fn build(source: &SourceSpec, width: u32, height: u32, fps: u32) -> anyhow::Result<Self>; pub fn start(&self, on_sample: Box<dyn Fn(&[u8]) + Send + Sync + 'static>) -> anyhow::Result<()>; pub fn stop(&self); }`
  - `pub fn select_encoder_element(has_nvenc: bool) -> &'static str` (pure, tested).

- [ ] **Step 1: Write the failing test for encoder selection**

At bottom of `src/media.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_selection() {
        assert_eq!(select_encoder_element(true), "nvv4l2h264enc");
        assert_eq!(select_encoder_element(false), "x264enc");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --features media --bin flo encoder_selection 2>&1 | tail -15`
Expected: FAIL — `select_encoder_element` not defined.

- [ ] **Step 3: Implement `src/media.rs`**

```rust
//! GStreamer capture + hardware-accelerated encode for the WebRTC media pipeline.
//! Feature-gated: requires system GStreamer (>= 1.14 with x264enc/h264parse/videotestsrc;
//! nvv4l2h264enc on Jetson). webrtc-rs owns the PeerConnection; this module only
//! produces encoded H.264 sample bytes via appsink.

#![cfg(feature = "media")]

use anyhow::{anyhow, Context, Result};
use std::sync::Arc;

use gstreamer::prelude::*;
use gstreamer_app::AppSink;

use crate::codec::Codec;

/// Where the video frames come from.
pub enum SourceSpec {
    /// Synthetic test pattern (no camera needed for the demo).
    Videotest,
    /// A V4L2 device, e.g. "/dev/video0".
    V4l2(String),
}

/// Pick the H.264 encoder element. Jetson has `nvv4l2h264enc` (NVENC, zero-copy
/// NVMM); everywhere else we fall back to `x264enc`. Pure + testable.
pub fn select_encoder_element(has_nvenc: bool) -> &'static str {
    if has_nvenc { "nvv4l2h264enc" } else { "x264enc" }
}

/// A running GStreamer encode pipeline that hands encoded bytes to a callback.
pub struct MediaPipeline {
    pipeline: gstreamer::Pipeline,
}

impl MediaPipeline {
    /// Build the pipeline. `source` chooses the input; `width/height/fps` set caps.
    pub fn build(source: &SourceSpec, width: u32, height: u32, fps: u32) -> Result<Self> {
        gstreamer::init().context("gstreamer init")?;

        let src = match source {
            SourceSpec::Videotest => format!(
                "videotestsrc is-live=true pattern=ball ! video/x-raw,format=NV12,width={width},height={height},framerate={fps}/1"
            ),
            SourceSpec::V4l2(dev) => format!(
                "v4l2src device={dev} ! video/x-raw,format=NV12,width={width},height={height},framerate={fps}/1"
            ),
        };

        let has_nvenc = gstreamer::ElementFactory::find("nvv4l2h264enc").is_some();
        let enc = select_encoder_element(has_nvenc);
        tracing::info!(encoder = enc, "building media pipeline");

        let desc = format!(
            "{src} ! videoconvert ! {enc} ! h264parse ! appsink name=enc drop=true max-buffers=2"
        );
        let pipeline = gstreamer::parse_launch(&desc)
            .context("parse_launch media pipeline")?
            .downcast::<gstreamer::Pipeline>()
            .map_err(|_| anyhow!("media pipeline is not a Pipeline"))?;

        Ok(Self { pipeline })
    }

    /// Start the pipeline; each encoded H.264 sample is delivered to `on_sample`.
    pub fn start(&self, on_sample: Box<dyn Fn(&[u8]) + Send + Sync + 'static>) -> Result<()> {
        let appsink = self
            .pipeline
            .by_name("enc")
            .context("appsink 'enc' missing")?
            .downcast::<AppSink>()
            .map_err(|_| anyhow!("'enc' is not an AppSink"))?;

        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = match sink.pull_sample() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "appsink pull_sample failed");
                            return gstreamer::FlowSuccess::Ok;
                        }
                    };
                    if let Some(buffer) = sample.buffer() {
                        let map = buffer.map_readable();
                        if let Ok(map) = map {
                            on_sample(&map);
                        }
                    }
                    gstreamer::FlowSuccess::Ok
                })
                .build(),
        );

        self.pipeline
            .set_state(gstreamer::State::Playing)
            .context("set pipeline to Playing")?;
        Ok(())
    }

    /// Stop and free the pipeline.
    pub fn stop(&self) {
        let _ = self.pipeline.set_state(gstreamer::State::Null);
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --features media --bin flo encoder_selection 2>&1 | tail -15`
Expected: PASS. (Full pipeline build requires system GStreamer; not compiled in default CI.)

- [ ] **Step 5: Commit**

```bash
git add src/media.rs
git commit -m "feat(media): gstreamer encode pipeline with appsink + Jetson NVENC auto-select"
```

---

## Task 3: `src/video.rs` — webrtc-rs PeerConnection glue + SignalHandler

**Files:**
- Create: `src/video.rs` (always built)
- Test: `src/video.rs` — pure helper `build_offer_description` / codec capability round-trip (no network).

**Interfaces:**
- Consumes: `crate::codec::{Codec, h264_codec_capability}` (Task 1), `crate::transport::Transport`, `crate::signaling::{self, SignalHandler, SignalMessage, IceCandidate}`.
- Produces:
  - `pub async fn start_video(robot_id: &str, peer_id: &str, transport: &Transport) -> anyhow::Result<()>`
  - `pub struct VideoPeer { ... }` implementing `signaling::SignalHandler`.
  - `pub fn h264_track(id: String, stream_id: String) -> Arc<webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample>`

- [ ] **Step 1: Write the failing test for the H.264 track helper**

At bottom of `src/video.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_track_has_correct_codec() {
        let t = h264_track("cam0".into(), "stream0".into());
        assert_eq!(t.codec().mime_type, webrtc::api::media_engine::MIME_TYPE_H264);
        assert_eq!(t.codec().clock_rate, 90_000);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin flo h264_track_has_correct_codec 2>&1 | tail -15`
Expected: FAIL — `h264_track` not defined.

- [ ] **Step 3: Implement `src/video.rs`**

```rust
//! WebRTC PeerConnection glue for class-3 video. webrtc-rs owns ICE/DTLS/RTP.
//! Signaling rides the existing zenoh mesh via `signaling` (unchanged schema).

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::codec::h264_codec_capability;
use crate::signaling::{IceCandidate, SignalHandler, SignalKind, SignalMessage};
use crate::transport::Transport;

/// Build an H.264 track local (clock rate 90 kHz) for webrtc-rs.
pub fn h264_track(id: String, stream_id: String) -> Arc<TrackLocalStaticSample> {
    Arc::new(TrackLocalStaticSample::new(h264_codec_capability(), id, stream_id))
}

/// State for one outbound video call. Implements `SignalHandler` so inbound
/// answers/ICE from the peer are applied to this PeerConnection.
pub struct VideoPeer {
    robot_id: String,
    peer_id: String,
    pc: Arc<RTCPeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    transport: Transport,
}

impl VideoPeer {
    /// Create the PC, add the H.264 track, wire ICE + offer, and publish the offer.
    pub async fn offer(
        robot_id: &str,
        peer_id: &str,
        transport: &Transport,
    ) -> Result<Arc<Self>> {
        let api = APIBuilder::new().build();
        let pc = Arc::new(
            api.new_peer_connection(RTCConfiguration::default())
                .await
                .context("new_peer_connection")?,
        );

        let track = h264_track(format!("{robot_id}-cam0"), format!("{robot_id}-stream0"));
        pc.add_track(track.clone()).await.context("add_track")?;

        // Trickle ICE candidates to the peer over zenoh.
        let t_robot = robot_id.to_string();
        let t_peer = peer_id.to_string();
        let t_tr = transport.clone();
        pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate> {
            let t_robot = t_robot.clone();
            let t_peer = t_peer.clone();
            let t_tr = t_tr.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    if let Ok(init) = c.to_json() {
                        let ice = IceCandidate {
                            candidate: init.candidate,
                            sdp_mid: init.sdp_mid,
                            mline_index: init.sdp_mline_index,
                        };
                        if let Err(e) = signaling::publish_ice(&t_tr, &t_robot, &t_peer, ice).await {
                            warn!(error = %e, "publish_ice failed");
                        }
                    }
                }
            })
        }));

        // Log inbound tracks (render is out of scope for v1).
        pc.on_track(Box::new(move |_track, _receiver, _transceiver| {
            info!(from = peer_id, "▶ video track received");
            Box::pin(async {})
        }));

        // Create + publish the offer.
        let offer = pc.create_offer(None).await.context("create_offer")?;
        pc.set_local_description(offer.clone()).await.context("set_local_description")?;
        signaling::publish_offer(transport, robot_id, peer_id, offer.sdp.clone(), vec![])
            .await
            .context("publish_offer")?;
        info!(robot_id, peer_id, "video offer published");

        Ok(Arc::new(Self {
            robot_id: robot_id.to_string(),
            peer_id: peer_id.to_string(),
            pc,
            track,
            transport: transport.clone(),
        }))
    }
}

impl SignalHandler for VideoPeer {
    fn on_answer(&self, _from: &str, msg: &SignalMessage) {
        let pc = self.pc.clone();
        let desc = RTCSessionDescription::answer(msg.sdp.clone()).expect("valid answer sdp");
        tokio::spawn(async move {
            if let Err(e) = pc.set_remote_description(desc).await {
                warn!(error = %e, "set_remote_description(answer) failed");
            }
        });
    }

    fn on_ice(&self, _from: &str, candidate: &IceCandidate) {
        let pc = self.pc.clone();
        let init = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.mline_index,
            username_fragment: None,
        };
        tokio::spawn(async move {
            if let Err(e) = pc.add_ice_candidate(init).await {
                warn!(error = %e, "add_ice_candidate failed");
            }
        });
    }

    fn on_offer(&self, _from: &str, _msg: &SignalMessage) {
        // v1 is offerer-initiated; answerer role is a later map. Ignore.
    }
}

/// Entry point called from `main` when `--video-peer` is set.
pub async fn start_video(robot_id: &str, peer_id: &str, transport: &Transport) -> Result<()> {
    let _peer = VideoPeer::offer(robot_id, peer_id, transport).await?;
    // Keep `_peer` alive for the process lifetime; signaling subscriptions hold Arc.
    std::mem::forget(_peer);
    Ok(())
}
```

Note: `RTCIceCandidateInit` field names — `sdp_mline_index` (Option<u16>) and `username_fragment`. Confirm against the pinned `webrtc` 0.17 source if a compile error appears; the field order/naming is stable in 0.17.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin flo h264_track_has_correct_codec 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/video.rs
git commit -m "feat: webrtc-rs PeerConnection glue + zenoh SignalHandler (no TURN)"
```

---

## Task 4: Wire video into `main.rs` + self-test mode

**Files:**
- Modify: `src/main.rs` (`run_demo` / `run_production` / `main` entry; add `--video-self-test`)
- Test: manual (requires system GStreamer via `--features media`).

**Interfaces:**
- Consumes: `crate::video::start_video` (Task 3), `crate::media::MediaPipeline` + `crate::media::SourceSpec` (Task 2, feature-gated), `crate::codec::Codec` (Task 1).

- [ ] **Step 1: Add gstreamer init + self-test in `main.rs`**

After `main()` parses `args`, branch:

```rust
#[cfg(feature = "media")]
if args.video.self_test {
    return run_video_self_test(&args.video.codec);
}
```

Implement `run_video_self_test` (feature-gated):

```rust
#[cfg(feature = "media")]
fn run_video_self_test(_codec: &crate::codec::Codec) -> anyhow::Result<()> {
    use crate::media::{MediaPipeline, SourceSpec};
    let pipeline = MediaPipeline::build(&SourceSpec::Videotest, 1280, 720, 30)?;
    let found = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = found.clone();
    pipeline.start(Box::new(move |bytes: &[u8]| {
        // Annex-B start code: 00 00 00 01
        if bytes.windows(4).any(|w| w == [0x00, 0x00, 0x00, 0x01]) {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::info!(len = bytes.len(), "▶ encoded H.264 sample (Annex-B start code ok)");
        }
    }))?;
    // Run a few seconds to pull samples.
    std::thread::sleep(std::time::Duration::from_secs(3));
    pipeline.stop();
    anyhow::ensure!(
        found.load(std::sync::atomic::Ordering::SeqCst),
        "no encoded H.264 samples produced"
    );
    println!("SELF-TEST OK: gstreamer encode produced H.264");
    Ok(())
}
```

- [ ] **Step 2: Launch `start_video` when `--video-peer` is set**

In `run_demo` (and `run_production`), after subsystems start, add:

```rust
if let Some(peer) = &args.video.peer {
    let tr = transport.clone();
    let rid = robot_id.clone();
    let pid = peer.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::video::start_video(&rid, &pid, &tr).await {
            tracing::error!(error = %e, "video failed");
        }
    });
}
```

- [ ] **Step 3: Build (default, no media) stays green**

Run: `cargo clippy --all-targets 2>&1 | grep -E "^error|^warning" | head`
Expected: empty.

- [ ] **Step 4: Build with media feature (requires system GStreamer)**

Run: `cargo clippy --features media --all-targets 2>&1 | grep -E "^error|^warning" | head`
Expected: empty when GStreamer dev libs are installed. If this env lacks them, document and skip compile here (Task 5 covers docs).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire --video-peer into main + --video-self-test (media feature)"
```

---

## Task 5: Docs + GETTING_STARTED + final verification

**Files:**
- Modify: `GETTING_STARTED.md`
- Modify: `docs/superpowers/specs/2026-07-18-webrtc-media-pipeline-design.md` (append implementation note, optional)
- Test: none new; final `cargo clippy` (default) + doc review.

**Interfaces:**
- Consumes: all prior tasks.

- [ ] **Step 1: Add the video section to `GETTING_STARTED.md`**

Append before "## Safety note":

```markdown
## Streaming live video (class-3, WebRTC)

`flo` can stream robot camera video peer-to-peer over WebRTC. GStreamer does the
capture + **hardware-accelerated encode** (NVENC `nvv4l2h264enc` on Jetson, `x264enc`
on a dev laptop); webrtc-rs owns the peer connection. Signaling rides the same
zenoh mesh as everything else — no separate service.

Prerequisites (only needed for video):

- System GStreamer >= 1.14 with `x264enc`, `h264parse`, `videotestsrc`
  (apt: `gstreamer1.0-plugins-{base,good,bad,ugly} gstreamer1.0-libav`).
  On Jetson, the NVIDIA accelerated GStreamer packages provide `nvv4l2h264enc`.
- Build with the `media` feature: `cargo build --features media`.

Two terminals, two nodes, real video:

```bash
# terminal 1
cargo run --features media --robot-id 7 --video-peer 8
# terminal 2
cargo run --features media --robot-id 8 --video-peer 7
```

Node 7 captures (synthetic pattern unless `--video-device /dev/video0`), encodes
H.264, and offers a WebRTC call to 8 over zenoh; 8 answers; video flows 7→8.
Node 8 logs `▶ video track received`. No camera? The demo uses `videotestsrc`.

Headless encode check (no peer needed):

```bash
cargo run --features media --video-self-test
```

It builds a GStreamer pipeline against `videotestsrc`, pulls encoded samples, and
asserts valid H.264 (Annex-B start code). Great for verifying Jetson HW encode.

Flags: `--video-peer <id>` (who to call), `--video-device <path>` (real camera;
default = synthetic), `--video-codec h264` (only `h264` in v1), `--video-self-test`.
```

- [ ] **Step 2: Final clippy (default features) + build**

Run: `cargo clippy --all-targets 2>&1 | grep -E "^error|^warning" | head`
Expected: empty. Confirm `cargo build` still green without GStreamer.

- [ ] **Step 3: Commit**

```bash
git add GETTING_STARTED.md
git commit -m "docs: document WebRTC video streaming + self-test in GETTING_STARTED"
```

---

## Self-Review Notes (executed by planner)

1. **Spec coverage** — encoder-only GStreamer (Task 2), webrtc-rs PC (Task 3), reused
   zenoh signaling (Task 3 uses `signaling::publish_*`), H.264 default + auto NVENC
   (Task 2 `select_encoder_element`), videotestsrc demo + 2-node P2P (Task 4/5),
   self-test (Task 4), out-of-scope items left out. All covered.
2. **Placeholders** — none. Every step has concrete code/commands.
3. **Type consistency** — `Codec` (Task 1) used in Tasks 2/4; `h264_codec_capability`
   used in Task 3; `MediaPipeline`/`SourceSpec` (Task 2) used in Task 4;
   `video::start_video` (Task 3) used in Task 4. `RTCIceCandidateInit` field names
   (`candidate`, `sdp_mid`, `sdp_mline_index`, `username_fragment`) match webrtc 0.17
   source verified during planning.
4. **Env caveat** — this environment has no system GStreamer, so `--features media`
   cannot be compiled here; default build stays green. Compilation of the media
   feature is verified on a machine with GStreamer (the user's Jetson/dev laptop).
