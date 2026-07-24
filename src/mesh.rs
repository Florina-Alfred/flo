//! Always-on WebRTC mesh signaling listener.
//!
//! [`MeshSignalHandler`] subscribes to inbound signaling for this robot and
//! auto-answers offers, so connectivity is two-way: whichever peer initiates,
//! the other side establishes its own `PeerConnection` and streams media back.
//! [`run_signaling`] wires presence + the receiver onto the zenoh transport.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{info, warn};

use crate::signaling::{self, IceCandidate, SignalHandler, SignalMessage};
use crate::transport::Transport;

/// Signal handler for the always-on mesh listener. Unlike the one-shot
/// [`flo_rs::video::start_video`] initiator, this answers inbound offers from any
/// peer by lazily creating a `VideoPeer` per peer and delegating to its
/// `SignalHandler` impl. When a capture `source` is configured it also starts
/// media capture on each answering peer, so the answerer streams video back
/// (two-way media).
#[derive(Clone)]
pub struct MeshSignalHandler {
    inner: Arc<MeshSignalHandlerInner>,
}

struct MeshSignalHandlerInner {
    robot_id: String,
    transport: Arc<Transport>,
    /// One answering PeerConnection per remote peer. Created on first inbound
    /// offer; reused for subsequent signaling with that peer.
    peers: Mutex<HashMap<String, Arc<flo_rs::video::VideoPeer>>>,
    /// Capture source for answering peers; `None` means "receive-only / no
    /// outbound media".
    source: Option<flo_rs::media::SourceSpec>,
}

impl MeshSignalHandler {
    pub fn new(
        robot_id: &str,
        transport: Arc<Transport>,
        source: Option<flo_rs::media::SourceSpec>,
    ) -> Self {
        Self {
            inner: Arc::new(MeshSignalHandlerInner {
                robot_id: robot_id.to_string(),
                transport,
                peers: Mutex::new(HashMap::new()),
                source,
            }),
        }
    }

    /// Synchronous lookup — avoids holding a non-Send `MutexGuard` across
    /// await points when called from spawned tasks.
    fn find_peer(&self, from: &str) -> Option<Arc<flo_rs::video::VideoPeer>> {
        match self.inner.peers.lock() {
            Ok(g) => g.get(from).cloned(),
            Err(e) => {
                warn!(error = %e, "peers lock poisoned in find_peer");
                None
            }
        }
    }

    /// Get the existing answering peer for `from`, or create one. The creation
    /// await happens outside the lock to avoid blocking other signaling.
    async fn peer_for(&self, from: &str) -> Option<Arc<flo_rs::video::VideoPeer>> {
        if let Some(p) = self.find_peer(from) {
            return Some(p);
        }
        let peer = match flo_rs::video::VideoPeer::answer(
            &self.inner.robot_id,
            from,
            self.inner.transport.clone(),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, peer = from, "failed to create answering PeerConnection");
                return None;
            }
        };
        let mut g = match self.inner.peers.lock() {
            Ok(g) => g,
            Err(e) => {
                warn!(error = %e, "peers lock poisoned in peer_for");
                return None;
            }
        };
        let was_new = !g.contains_key(&from.to_string());
        let peer = g.entry(from.to_string()).or_insert(peer).clone();
        if was_new && let Some(source) = self.inner.source.clone() {
            let p = peer.clone();
            let from = from.to_string();
            tokio::spawn(async move {
                if let Err(e) = flo_rs::video::start_capture(p, source, 1280, 720, 30).await {
                    warn!(error = %e, peer = from, "answerer capture failed to start");
                }
            });
        }
        Some(peer)
    }
}

impl SignalHandler for MeshSignalHandler {
    fn on_offer(&self, from: &str, msg: &SignalMessage) {
        let h = self.clone();
        let from = from.to_string();
        let msg = msg.clone();
        tokio::spawn(async move {
            if let Some(peer) = h.peer_for(&from).await {
                peer.on_offer(&from, &msg);
            }
        });
    }
    fn on_answer(&self, from: &str, msg: &SignalMessage) {
        let h = self.clone();
        let from = from.to_string();
        let msg = msg.clone();
        tokio::spawn(async move {
            let peers = match h.inner.peers.lock() {
                Ok(g) => g,
                Err(e) => {
                    warn!(error = %e, "peers lock poisoned, skipping answer");
                    return;
                }
            };
            let peer = match peers.get(&from).cloned() {
                Some(p) => p,
                None => return,
            };
            drop(peers);
            peer.on_answer(&from, &msg);
        });
    }
    fn on_ice(&self, from: &str, candidate: &IceCandidate) {
        let h = self.clone();
        let from = from.to_string();
        let candidate = candidate.clone();
        tokio::spawn(async move {
            let peers = match h.inner.peers.lock() {
                Ok(g) => g,
                Err(e) => {
                    warn!(error = %e, "peers lock poisoned, skipping ice");
                    return;
                }
            };
            let peer = match peers.get(&from).cloned() {
                Some(p) => p,
                None => return,
            };
            drop(peers);
            peer.on_ice(&from, &candidate);
        });
    }
}

/// Publish presence, subscribe to peer discovery, and run the inbound signal
/// receiver backed by [`MeshSignalHandler`] so the robot can both initiate and
/// answer WebRTC calls.
pub async fn run_signaling(
    transport: Arc<Transport>,
    robot_id: &str,
    source: Option<flo_rs::media::SourceSpec>,
) -> zenoh::Result<()> {
    signaling::publish_presence(
        &transport,
        robot_id,
        vec![format!("robot/{robot_id}/local/cam0")],
    )
    .await?;
    signaling::subscribe_presence(&transport, |p: signaling::Presence| {
        info!(peer = %p.id, streams = ?p.streams, "discovered peer");
    })
    .await?;
    let handler = MeshSignalHandler::new(robot_id, transport.clone(), source);
    signaling::run_signal_receiver(&transport, robot_id, handler).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end proof that the always-on mesh listener actually answers an
    /// inbound offer (the two-way half of WebRTC connectivity): open a loopback
    /// transport, attach `MeshSignalHandler`, publish an offer from a fake peer,
    /// and assert an answer is published back on the expected key-expr.
    #[tokio::test(flavor = "multi_thread")]
    async fn mesh_handler_answers_inbound_offer() {
        use std::sync::Arc;

        // Roles: `offerer` (robot7) opens a real PeerConnection and publishes a
        // valid offer; `answerer` (peer8) hosts the always-on MeshSignalHandler
        // which must auto-create its own PeerConnection and publish an answer.
        let offerer = "robot7";
        let answerer = "peer8";
        let transport = Arc::new(
            Transport::open_with(Transport::loopback_config())
                .await
                .expect("open loopback transport"),
        );

        // The answerer side: always-on mesh listener.
        let handler = MeshSignalHandler::new(answerer, transport.clone(), None);
        flo_rs::signaling::run_signal_receiver(&transport, answerer, handler)
            .await
            .expect("signal receiver");

        // Subscribe to the answer the answerer should publish back.
        // Key layout: robot/{answerer}/signal/{offerer}/answer.
        let answer_key = format!("robot/{answerer}/signal/{offerer}/answer");
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        transport
            .subscribe(&answer_key, move |s: zenoh::sample::Sample| {
                if let Some(tx) = tx.lock().expect("test lock poisoned").take() {
                    let _ = tx.send(s.payload().to_bytes().to_vec());
                }
            })
            .await
            .expect("subscribe answer key");

        // The offerer side: a real PeerConnection producing a valid SDP offer.
        // `VideoPeer::offer` publishes the offer over the same transport, which
        // the answerer's mesh listener receives.
        let _offerer = flo_rs::video::VideoPeer::offer(offerer, answerer, transport.clone())
            .await
            .expect("offerer PeerConnection");

        // The handler must auto-create an answering PeerConnection and publish
        // an answer within a few seconds.
        let got = tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .expect("answer within 15s")
            .expect("answer payload");
        let v: serde_json::Value = serde_json::from_slice(&got).expect("answer is JSON");
        assert_eq!(v["kind"], "answer");
        assert_eq!(v["from"], answerer);
        assert_eq!(v["to"], offerer);
    }
}
