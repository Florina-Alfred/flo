//! GStreamer capture + hardware-accelerated encode for the WebRTC media pipeline.
//! Feature-gated: requires system GStreamer (>= 1.14 with x264enc/h264parse/videotestsrc;
//! nvv4l2h264enc on Jetson). webrtc-rs owns the PeerConnection; this module only
//! produces encoded H.264 sample bytes via appsink.

#![cfg(feature = "media")]

use anyhow::{anyhow, Context, Result};

use gstreamer::prelude::*;
use gstreamer_app::AppSink;

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
                            return Ok(gstreamer::FlowSuccess::Ok);
                        }
                    };
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
    use super::*;

    #[test]
    fn encoder_selection() {
        assert_eq!(select_encoder_element(true), "nvv4l2h264enc");
        assert_eq!(select_encoder_element(false), "x264enc");
    }
}
