//! WebRTC PeerConnection glue for class-3 video. webrtc-rs owns ICE/DTLS/RTP.
//! Signaling rides the existing zenoh mesh via `signaling` (unchanged schema).

use std::sync::Arc;

use anyhow::Context;
use tracing::{info, warn};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::codec::h264_codec_capability;
use crate::signaling::{IceCandidate, SignalHandler, SignalMessage};

/// Build an H.264 track local (clock rate 90 kHz) for webrtc-rs.
pub fn h264_track(id: String, stream_id: String) -> Arc<TrackLocalStaticSample> {
    Arc::new(TrackLocalStaticSample::new(
        h264_codec_capability(),
        id,
        stream_id,
    ))
}

/// State for one outbound video call. Implements `SignalHandler` so inbound
/// answers/ICE from the peer are applied to this PeerConnection.
pub struct VideoPeer {
    robot_id: String,
    /// Remote peer id (set on construction; retained for diagnostics/logging).
    #[allow(dead_code)]
    peer_id: String,
    pc: Arc<RTCPeerConnection>,
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    track: Arc<TrackLocalStaticSample>,
    transport: Arc<crate::transport::Transport>,
    /// Optional consumer hook invoked when an inbound track arrives. Defaults to
    /// logging; a render-free "forward" consumer can attach a reader here.
    #[allow(dead_code)]
    on_track: Arc<std::sync::Mutex<Option<OnTrack>>>,
}

/// Callback invoked when a remote track is received on this peer. Receives the
/// inbound `TrackRemote` so a consumer can attach a sample reader; `flo` itself
/// does no rendering.
pub type OnTrack = Arc<dyn Fn(Arc<webrtc::track::track_remote::TrackRemote>) + Send + Sync>;

impl VideoPeer {
    /// Build the `PeerConnection`, add the H.264 track, and wire trickle-ICE so
    /// candidates are relayed to the peer over zenoh. Shared by [`VideoPeer::offer`] and
    /// [`VideoPeer::answer`]; neither creates nor publishes an SDP here.
    async fn build(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<(
        Arc<RTCPeerConnection>,
        Arc<TrackLocalStaticSample>,
        Arc<std::sync::Mutex<Option<OnTrack>>>,
    )> {
        // Register the H.264 codec in the MediaEngine so `add_track` has a codec
        // to populate the SDP media section with (webrtc-rs rejects an
        // RTPSender with no registered codec). Without this, offer/answer
        // creation fails with "RTPSender created with no codecs".
        let mut media_engine = webrtc::api::media_engine::MediaEngine::default();
        media_engine
            .register_codec(
                webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters {
                    capability: h264_codec_capability(),
                    payload_type: 102,
                    stats_id: String::new(),
                },
                webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video,
            )
            .context("register h264 codec")?;
        let api = APIBuilder::new().with_media_engine(media_engine).build();
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
        pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
            let t_robot = t_robot.clone();
            let t_peer = t_peer.clone();
            let t_tr = t_tr.clone();
            Box::pin(async move {
                if let Some(c) = c
                    && let Ok(init) = c.to_json()
                {
                    let ice = IceCandidate {
                        candidate: init.candidate,
                        sdp_mid: init.sdp_mid,
                        mline_index: init.sdp_mline_index,
                    };
                    if let Err(e) =
                        crate::signaling::publish_ice(&t_tr, &t_robot, &t_peer, ice).await
                    {
                        warn!(error = %e, "publish_ice failed");
                    }
                }
            })
        }));

        // Inbound tracks: deliver to the user callback if registered, else log.
        // `flo` performs no rendering; a consumer attaches a reader here.
        let on_track: Arc<std::sync::Mutex<Option<OnTrack>>> = Default::default();
        let cb = on_track.clone();
        let log_peer = peer_id.to_string();
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let cb = cb.clone();
            let log_peer = log_peer.clone();
            Box::pin(async move {
                info!(from = %log_peer, "▶ video track received");
                if let Some(f) = cb.lock().unwrap().clone() {
                    f(track);
                }
            })
        }));

        Ok((pc, track, on_track))
    }

    /// Create the PC, add the H.264 track, wire ICE, create+publish an offer.
    /// Use this on the side that initiates the call.
    pub async fn offer(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<Arc<Self>> {
        let (pc, track, on_track) = Self::build(robot_id, peer_id, transport.clone()).await?;

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
            peer_id: peer_id.to_string(),
            pc,
            track,
            transport,
            on_track,
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
        let (pc, track, on_track) = Self::build(robot_id, peer_id, transport.clone()).await?;
        info!(robot_id, peer_id, "video responder PeerConnection ready");
        Ok(Arc::new(Self {
            robot_id: robot_id.to_string(),
            peer_id: peer_id.to_string(),
            pc,
            track,
            transport,
            on_track,
        }))
    }

    /// Borrow the outbound track so a media pipeline can push encoded samples.
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    pub fn track(&self) -> Arc<TrackLocalStaticSample> {
        self.track.clone()
    }

    /// Register a callback invoked when an inbound track arrives. Default (until
    /// set) is to log only; pass a closure to forward/consume the remote track.
    #[allow(dead_code)]
    pub fn set_on_track(&self, cb: OnTrack) {
        *self.on_track.lock().unwrap() = Some(cb);
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
    crate::signaling::run_signal_receiver(&transport, robot_id, peer.clone())
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
    use webrtc::media::Sample as MediaSample;

    let pipeline = MediaPipeline::build(&source, width, height, fps)?;
    let track = peer.track();
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let ticks_per_frame = 90_000 / fps.max(1);
    pipeline.start(Box::new(move |bytes: &[u8]| {
        let ts = counter.fetch_add(ticks_per_frame, std::sync::atomic::Ordering::SeqCst);
        let track = track.clone();
        let sample = MediaSample {
            data: bytes::Bytes::copy_from_slice(bytes),
            timestamp: std::time::SystemTime::now(),
            duration: std::time::Duration::from_secs_f64(1.0 / fps as f64),
            packet_timestamp: ts,
            ..Default::default()
        };
        tokio::spawn(async move {
            if let Err(e) = track.write_sample(&sample).await {
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
    #[cfg(feature = "media")]
    use webrtc::track::track_local::TrackLocal;

    #[test]
    fn h264_track_has_correct_codec() {
        let t = h264_track("cam0".into(), "stream0".into());
        assert_eq!(
            t.codec().mime_type,
            webrtc::api::media_engine::MIME_TYPE_H264
        );
        assert_eq!(t.codec().clock_rate, 90_000);
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
        assert_eq!(peer.track().id(), format!("{}-cam0", "robot7"));

        // Capture must start cleanly on the answerer (the Phase 2 wiring).
        start_capture(peer, crate::media::SourceSpec::Videotest, 1280, 720, 30)
            .await
            .expect("answerer capture starts");
    }
}
