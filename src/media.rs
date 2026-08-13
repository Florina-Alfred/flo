//! GStreamer capture + hardware-accelerated encode for the WebRTC media pipeline.
//! Feature-gated: requires system GStreamer (>= 1.14 with x264enc/h264parse/videotestsrc;
//! nvv4l2h264enc on Jetson). webrtc-rs owns the PeerConnection; this module only
//! produces encoded H.264 sample bytes via appsink.

#![cfg(feature = "media")]

use anyhow::{Context, Result, anyhow};

use gstreamer::prelude::*;
use gstreamer_app::AppSink;

/// Where the video frames come from.
#[derive(Clone)]
pub enum SourceSpec {
    /// Synthetic test pattern (no camera needed for the demo).
    Videotest,
    /// A V4L2 device, e.g. "/dev/video0".
    V4l2(String),
}

/// Pick the H.264 encoder element name. Jetson has `nvv4l2h264enc` (NVENC, zero-copy
/// NVMM); everywhere else we fall back to `x264enc`. Pure + testable.
pub fn encoder_element_name(has_nvenc: bool) -> &'static str {
    if has_nvenc {
        "nvv4l2h264enc"
    } else {
        "x264enc"
    }
}

/// A running GStreamer encode pipeline that hands encoded bytes to a callback.
pub struct MediaPipeline {
    pipeline: gstreamer::Pipeline,
}

/// Sample callback: receives each encoded H.264 frame as raw bytes.
pub type SampleCallback = Box<dyn Fn(&[u8]) + Send + Sync + 'static>;

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
        let enc = encoder_element_name(has_nvenc);
        tracing::info!(encoder = enc, "building media pipeline");

        let desc = format!(
            "{src} ! videoconvert ! {enc} ! h264parse ! appsink name=enc drop=true max-buffers=2"
        );
        let pipeline = gstreamer::parse::launch(&desc)
            .context("parse_launch media pipeline")?
            .downcast::<gstreamer::Pipeline>()
            .map_err(|_| anyhow!("media pipeline is not a Pipeline"))?;

        Ok(Self { pipeline })
    }

    /// Start the pipeline; each encoded H.264 sample is delivered to `on_sample`.
    pub fn start(&self, on_sample: SampleCallback) -> Result<()> {
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
                            return Ok(gstreamer::FlowSuccess::Ok);
                        }
                    };
                    #[allow(clippy::collapsible_if)]
                    if let Some(buffer) = sample.buffer() {
                        if let Ok(map) = buffer.map_readable() {
                            on_sample(&map);
                        }
                    }
                    Ok(gstreamer::FlowSuccess::Ok)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn encoder_selection() {
        assert_eq!(encoder_element_name(true), "nvv4l2h264enc");
        assert_eq!(encoder_element_name(false), "x264enc");
    }

    #[test]
    fn videotest_pipeline_builds_and_reaches_playing() {
        let pipeline = MediaPipeline::build(&SourceSpec::Videotest, 320, 240, 10)
            .expect("build videotest pipeline");
        let samples = Arc::new(AtomicU64::new(0));
        let count = samples.clone();
        pipeline
            .start(Box::new(move |_frame| {
                count.fetch_add(1, Ordering::SeqCst);
            }))
            .expect("start pipeline");

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if pipeline.pipeline.current_state() == gstreamer::State::Playing
                && samples.load(Ordering::SeqCst) > 0
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "pipeline never reached Playing (state={:?}, samples={:?})",
                pipeline.pipeline.current_state(),
                samples.load(Ordering::SeqCst)
            );
            std::thread::sleep(Duration::from_millis(25));
        }

        pipeline.stop();
    }

    #[test]
    fn missing_v4l2_device_fails_cleanly() {
        // A non-existent device must fail as a clean GStreamer error, never panic.
        // The failure can surface on either axis: `set_state` may return Err
        // synchronously, or the element posts an Error message on the pipeline bus
        // asynchronously after an Async state-change reply. Accept whichever the
        // running gstreamer does, within a bounded window.
        let Some(pipeline) = MediaPipeline::build(
            &SourceSpec::V4l2("/dev/definitely-not-a-video-device".into()),
            320,
            240,
            10,
        )
        .ok() else {
            // No v4l2src factory on the host → build surfaced the failure; also clean.
            return;
        };

        let bus = pipeline.pipeline.bus().expect("pipeline bus");
        let start_failed = pipeline.start(Box::new(|_frame| {})).is_err();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if start_failed
                || bus
                    .timed_pop_filtered(
                        gstreamer::ClockTime::from_mseconds(25),
                        &[gstreamer::MessageType::Error],
                    )
                    .is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "missing device never surfaced as a bus Error within 10s"
            );
        }

        pipeline.stop();
    }
}
