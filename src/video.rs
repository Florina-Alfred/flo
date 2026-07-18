//! WebRTC PeerConnection glue for class-3 video. webrtc-rs owns ICE/DTLS/RTP.
//! Signaling rides the existing zenoh mesh via `signaling` (unchanged schema).

use std::sync::Arc;

use anyhow::Context;
use tracing::{info, warn};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
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
    #[allow(dead_code)]
    robot_id: String,
    #[allow(dead_code)]
    peer_id: String,
    pc: Arc<RTCPeerConnection>,
    #[allow(dead_code)]
    track: Arc<TrackLocalStaticSample>,
}

impl VideoPeer {
    /// Create the PC, add the H.264 track, wire ICE + offer, and publish the offer.
    pub async fn offer(
        robot_id: &str,
        peer_id: &str,
        transport: Arc<crate::transport::Transport>,
    ) -> anyhow::Result<Arc<Self>> {
        let api = APIBuilder::new().build();
        let pc = Arc::new(
            api.new_peer_connection(RTCConfiguration::default())
                .await
                .context("new_peer_connection")?,
        );

        let track = h264_track(
            format!("{robot_id}-cam0"),
            format!("{robot_id}-stream0"),
        );
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

        // Log inbound tracks (render is out of scope for v1).
        let log_peer = peer_id.to_string();
        pc.on_track(Box::new(move |_track, _receiver, _transceiver| {
            info!(from = %log_peer, "▶ video track received");
            Box::pin(async {})
        }));

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

    fn on_offer(&self, _from: &str, _msg: &SignalMessage) {
        // v1 is offerer-initiated; answerer role is a later map. Ignore.
    }
}

/// Entry point called from `main` when `--video-peer` is set.
pub async fn start_video(
    robot_id: &str,
    peer_id: &str,
    transport: Arc<crate::transport::Transport>,
) -> anyhow::Result<()> {
    let _peer = VideoPeer::offer(robot_id, peer_id, transport).await?;
    // Keep `_peer` alive for the process lifetime; signaling subscriptions hold Arc.
    std::mem::forget(_peer);
    Ok(())
}

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
