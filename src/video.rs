//! WebRTC PeerConnection glue for class-3 video. webrtc-rs owns ICE/DTLS/RTP.
//! Signaling rides the existing zenoh mesh via `signaling` (unchanged schema).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use anyhow::Context;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use tracing::{info, warn};
use webrtc::media_stream::Track;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceCandidateInit, RTCPeerConnectionIceEvent,
    RTCPeerConnectionState, RTCSessionDescription,
};
use webrtc::rtp_transceiver::RtpSender;

use crate::codec::h264_codec_parameters;
use crate::signaling::{IceCandidate, SignalHandler, SignalMessage};

/// Deterministic SSRC source: a simple counter (no `rand` dependency). Uniqueness
/// per peer is all that matters for the single outbound H.264 track.
static NEXT_SSRC: AtomicU32 = AtomicU32::new(0x5100_0000);

/// Build an H.264 track local (clock rate 90 kHz) for webrtc-rs.
pub fn h264_track(id: String, stream_id: String) -> Arc<TrackLocalStaticSample> {
    let ssrc = NEXT_SSRC.fetch_add(1, Ordering::Relaxed);
    let track = MediaStreamTrack::new(
        stream_id,
        id,
        "flo-h264".to_owned(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: h264_codec_parameters().rtp_codec,
            ..Default::default()
        }],
    );
    Arc::new(
        TrackLocalStaticSample::new(Instant::now(), track)
            .expect("H.264 codec has a registered payloader"),
    )
}

/// Async event handler wired into the `PeerConnectionBuilder`. Replaces the 0.17
/// closure callbacks: trickles ICE candidates to the peer over zenoh and logs
/// receive-side events (flo does not render inbound media).
struct VideoHandler {
    transport: Arc<crate::transport::Transport>,
    robot_id: String,
    peer_id: String,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for VideoHandler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        let Ok(init) = event.candidate.to_json() else {
            warn!("ice candidate to_json failed");
            return;
        };
        let ice = IceCandidate {
            candidate: init.candidate,
            sdp_mid: init.sdp_mid,
            mline_index: init.sdp_mline_index,
        };
        if let Err(e) =
            crate::signaling::publish_ice(&self.transport, &self.robot_id, &self.peer_id, ice).await
        {
            warn!(error = %e, "publish_ice failed");
        }
    }

    async fn on_track(&self, _track: Arc<dyn TrackRemote>) {
        info!(from = %self.peer_id, "▶ video track received");
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        info!(from = %self.peer_id, ?state, "peer connection state changed");
    }
}

/// State for one outbound video call. Implements `SignalHandler` so inbound
/// answers/ICE from the peer are applied to this PeerConnection.
pub struct VideoPeer {
    robot_id: String,
    pc: Arc<dyn PeerConnection>,
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    track: Arc<TrackLocalStaticSample>,
    /// Sender for the outbound track; used to resolve the negotiated payload
    /// type when starting capture.
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    sender: Arc<dyn RtpSender>,
    transport: Arc<crate::transport::Transport>,
}

impl VideoPeer {
    /// Build the `PeerConnection`, add the H.264 track, and wire trickle-ICE so
    /// candidates are relayed to the peer over zenoh. Shared by [`VideoPeer::offer`] and
    /// [`VideoPeer::answer`]; neither creates nor publishes an SDP here.
    async fn build(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<(
        Arc<dyn PeerConnection>,
        Arc<TrackLocalStaticSample>,
        Arc<dyn RtpSender>,
    )> {
        // Register the H.264 codec in the MediaEngine so `add_track` has a codec
        // to populate the SDP media section with (webrtc-rs rejects an
        // RTPSender with no registered codec). Without this, offer/answer
        // creation fails with "RTPSender created with no codecs".
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_codec(h264_codec_parameters(), RtpCodecKind::Video)
            .context("register h264 codec")?;
        let config = RTCConfigurationBuilder::new().build();
        let pc: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_configuration(config)
                .with_media_engine(media_engine)
                .with_handler(Arc::new(VideoHandler {
                    transport: transport.clone(),
                    robot_id: robot_id.to_string(),
                    peer_id: peer_id.to_string(),
                }))
                .with_udp_addrs(vec!["0.0.0.0:0"])
                .build()
                .await
                .context("new_peer_connection")?,
        );

        let track = h264_track(format!("{robot_id}-cam0"), format!("{robot_id}-stream0"));
        let sender = pc
            .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await
            .context("add_track")?;

        Ok((pc, track, sender))
    }

    /// Create the PC, add the H.264 track, wire ICE, create+publish an offer.
    /// Use this on the side that initiates the call.
    pub async fn offer(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<Arc<Self>> {
        let (pc, track, sender) = Self::build(robot_id, peer_id, transport.clone()).await?;

        // Create + publish the offer.
        let offer = pc.create_offer(None).await.context("create_offer")?;
        pc.set_local_description(offer.clone())
            .await
            .context("set_local_description")?;
        crate::signaling::publish_offer(&transport, robot_id, peer_id, offer.sdp.clone(), vec![])
            .await
            .map_err(|e| anyhow::anyhow!("publish_offer: {e}"))?;
        info!(robot_id, peer_id, "video offer published");

        Ok(Arc::new(Self {
            robot_id: robot_id.to_string(),
            pc,
            track,
            sender,
            transport,
        }))
    }

    /// Create the PC, add the H.264 track, and wire ICE — without sending an
    /// offer. Use this on the responding side: when an inbound offer arrives,
    /// [`SignalHandler::on_offer`] sets the remote description and publishes an
    /// answer on this same `PeerConnection`. This is what makes connectivity
    /// two-way (either peer can initiate; the other auto-answers).
    pub async fn answer(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<Arc<Self>> {
        let (pc, track, sender) = Self::build(robot_id, peer_id, transport.clone()).await?;
        info!(robot_id, peer_id, "video responder PeerConnection ready");
        Ok(Arc::new(Self {
            robot_id: robot_id.to_string(),
            pc,
            track,
            sender,
            transport,
        }))
    }

    /// Borrow the outbound track so a media pipeline can push encoded samples.
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    pub fn track(&self) -> Arc<TrackLocalStaticSample> {
        self.track.clone()
    }
}

impl SignalHandler for VideoPeer {
    fn on_answer(&self, _from: &str, msg: &SignalMessage) {
        let pc = self.pc.clone();
        if let Ok(desc) = RTCSessionDescription::answer(msg.sdp.clone()) {
            tokio::spawn(async move {
                if let Err(e) = pc.set_remote_description(desc).await {
                    warn!(error = %e, "set_remote_description(answer) failed");
                }
            });
        } else {
            warn!(sdp_len = msg.sdp.len(), "received malformed answer sdp");
        }
    }

    fn on_ice(&self, _from: &str, candidate: &IceCandidate) {
        let pc = self.pc.clone();
        let init = RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.mline_index,
            username_fragment: None,
            url: None,
        };
        tokio::spawn(async move {
            if let Err(e) = pc.add_ice_candidate(init).await {
                warn!(error = %e, "add_ice_candidate failed");
            }
        });
    }

    fn on_offer(&self, from: &str, msg: &SignalMessage) {
        let pc = self.pc.clone();
        let tr = self.transport.clone();
        let me = self.robot_id.clone();
        let from = from.to_string();
        let offer = match RTCSessionDescription::offer(msg.sdp.clone()) {
            Ok(o) => o,
            Err(e) => {
                warn!(error = %e, "bad offer sdp");
                return;
            }
        };
        tokio::spawn(async move {
            if let Err(e) = pc.set_remote_description(offer).await {
                warn!(error = %e, "set_remote_description(offer) failed");
                return;
            }
            let answer = match pc.create_answer(None).await {
                Ok(a) => a,
                Err(e) => {
                    warn!(error = %e, "create_answer failed");
                    return;
                }
            };
            if let Err(e) = pc.set_local_description(answer.clone()).await {
                warn!(error = %e, "set_local_description(answer) failed");
                return;
            }
            if let Err(e) =
                crate::signaling::publish_answer(&tr, &me, &from, answer.sdp, vec![]).await
            {
                warn!(error = %e, "publish_answer failed");
            }
        });
    }
}

/// Forwarding impl so an `Arc<VideoPeer>` satisfies `SignalHandler` for the
/// signal receiver (which holds the handler behind an `Arc`).
impl SignalHandler for Arc<VideoPeer> {
    fn on_offer(&self, from: &str, msg: &SignalMessage) {
        VideoPeer::on_offer(self, from, msg);
    }
    fn on_answer(&self, from: &str, msg: &SignalMessage) {
        VideoPeer::on_answer(self, from, msg);
    }
    fn on_ice(&self, from: &str, candidate: &IceCandidate) {
        VideoPeer::on_ice(self, from, candidate);
    }
}

/// Entry point called from `main` when `--video-peer` is set (no media capture).
///
/// Builds the offerer `VideoPeer`, then subscribes it (behind an `Arc`) to the
/// signal receiver. The receiver owns the `Arc`, keeping the peer alive for the
/// session; inbound answers/ICE are applied to its `PeerConnection`.
#[cfg(not(feature = "media"))]
pub async fn start_video(
    robot_id: &str,
    peer_id: &str,
    transport: Arc<crate::transport::Transport>,
) -> anyhow::Result<()> {
    let peer = VideoPeer::offer(robot_id, peer_id, transport.clone()).await?;
    crate::signaling::run_signal_receiver(&transport, robot_id, peer.clone())
        .await
        .map_err(|e| anyhow::anyhow!("signal receiver: {e}"))?;
    Ok(())
}

/// Like [`start_video`] but also starts a media capture pipeline that forwards
/// encoded H.264 samples into the peer's outbound track. Feature-gated: needs
/// system GStreamer.
#[cfg(feature = "media")]
pub async fn start_video_with_source(
    robot_id: &str,
    peer_id: &str,
    transport: Arc<crate::transport::Transport>,
    source: crate::media::SourceSpec,
) -> anyhow::Result<()> {
    let peer = VideoPeer::offer(robot_id, peer_id, transport.clone()).await?;
    // Start capture; `start_capture` leaks the GStreamer pipeline so it stays
    // alive for the daemon lifetime (appsink callbacks own the buffers).
    if let Err(e) = start_capture(peer.clone(), source, 1280, 720, 30).await {
        warn!(error = %e, "media capture failed to start");
    }
    crate::signaling::run_signal_receiver(&transport, robot_id, peer)
        .await
        .map_err(|e| anyhow::anyhow!("signal receiver: {e}"))?;
    Ok(())
}

/// Build a GStreamer encode pipeline and forward every encoded sample into the
/// peer's `TrackLocalStaticSample`. The pipeline is leaked (not dropped) so the
/// daemon keeps producing; this is the intended long-lived strategy for a robot
/// client. `MediaPipeline` itself is feature-gated.
#[cfg(feature = "media")]
pub async fn start_capture(
    peer: Arc<VideoPeer>,
    source: crate::media::SourceSpec,
    width: u32,
    height: u32,
    fps: u32,
) -> anyhow::Result<()> {
    use crate::media::MediaPipeline;
    use rtc::media::Sample;

    let pipeline = MediaPipeline::build(&source, width, height, fps)?;
    let track = peer.track();

    // Resolve the outbound SSRC and negotiated payload type once, up front.
    let ssrc = *track
        .ssrcs()
        .await
        .first()
        .expect("outbound track has an SSRC");
    // Prefer the negotiated payload type; fall back to the registered one (102)
    // until an offer/answer populates the sender's codecs (the answerer side can
    // start capture before any negotiation).
    let payload_type = peer
        .sender
        .get_parameters()
        .await?
        .rtp_parameters
        .codecs
        .first()
        .map(|c| c.payload_type)
        .unwrap_or_else(|| h264_codec_parameters().payload_type);

    let duration = std::time::Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    pipeline.start(Box::new(move |bytes: &[u8]| {
        let track = track.clone();
        let sample = Sample {
            data: bytes::Bytes::copy_from_slice(bytes),
            duration,
            ..Sample::new(Instant::now())
        };
        tokio::spawn(async move {
            if let Err(e) = track
                .sample_writer(ssrc, payload_type)
                .write_sample(&sample)
                .await
            {
                tracing::warn!(error = %e, "write_sample failed");
            }
        });
    }))?;

    // Keep the GStreamer pipeline alive for the process lifetime. The appsink
    // callbacks hold the encoded buffers; dropping the pipeline here would stop
    // the source immediately. A robot client is a long-lived daemon, so leaking
    // is the pragmatic choice.
    std::mem::forget(pipeline);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn h264_track_has_correct_codec() {
        let t = h264_track("cam0".into(), "stream0".into());
        let ssrc = *t.ssrcs().await.first().expect("one ssrc");
        let codec = t.codec(ssrc).await.expect("codec for ssrc");
        assert_eq!(
            codec.mime_type,
            rtc::peer_connection::configuration::media_engine::MIME_TYPE_H264
        );
        assert_eq!(codec.clock_rate, 90_000);
    }

    /// The answering side must be able to start a capture pipeline against its
    /// own outbound track (two-way media): building `VideoPeer::answer` and
    /// starting capture should succeed and produce an immediately writable track.
    #[cfg(feature = "media")]
    #[tokio::test(flavor = "multi_thread")]
    async fn answerer_can_start_capture() {
        use std::sync::Arc;

        let transport = Arc::new(
            crate::transport::Transport::open_with(crate::transport::Transport::loopback_config())
                .await
                .expect("open loopback transport"),
        );
        let peer = VideoPeer::answer("robot7", "peer8", transport)
            .await
            .expect("answering PeerConnection");
        // The outbound track is usable before any remote description is set.
        assert_eq!(peer.track().track_id().await, format!("{}-cam0", "robot7"));

        // Capture must start cleanly on the answerer (the Phase 2 wiring).
        start_capture(peer, crate::media::SourceSpec::Videotest, 1280, 720, 30)
            .await
            .expect("answerer capture starts");
    }
}
