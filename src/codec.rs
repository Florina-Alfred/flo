//! Video codec selection. Pure, no GStreamer — unit-testable.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// H.264 — default. Hardware (nvv4l2h264enc) on Jetson, x264enc on dev.
    H264,
    // Av1, Vp8 reserved for a later release.
}

#[allow(clippy::derivable_impls)]
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
            other => Err(format!(
                "unsupported --video-codec '{other}' (v1 supports: h264)"
            )),
        }
    }
}

/// Build the webrtc-rs codec parameters for H.264 (clock rate 90 kHz, per RFC 6184).
#[cfg(feature = "media")]
pub fn h264_codec_parameters() -> rtc::rtp_transceiver::rtp_sender::RTCRtpCodecParameters {
    rtc::rtp_transceiver::rtp_sender::RTCRtpCodecParameters {
        rtp_codec: rtc::rtp_transceiver::rtp_sender::RTCRtpCodec {
            mime_type: rtc::peer_connection::configuration::media_engine::MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 102,
    }
}
