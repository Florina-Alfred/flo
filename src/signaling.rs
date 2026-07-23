//! WebRTC signaling over the Zenoh mesh (class-3 video).
//!
//! Locked design (webrtc-signaling map): signaling rides the `flo` Zenoh session;
//! peers discover each other via a presence key-expr; the offer/answer/ICE exchange
//! uses per-peer key-exprs with a minimal JSON envelope. No media is handled here —
//! this module only performs the SDP/ICE handshake so a later module can attach
//! webrtc-rs peer connections.
//!
//! The publisher functions ([`publish_offer`], [`publish_answer`], [`publish_ice`])
//! and the offer/answer/ICE key-exprs are driven by the `video`/`mesh` modules:
//! `VideoPeer` publishes offers/answers/ICE over this transport, and
//! `MeshSignalHandler` runs the inbound receiver. No media is handled here — this
//! module only performs the SDP/ICE handshake so `video` can attach peer
//! connections.

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::transport::Transport;

/// A trickled ICE candidate, carried opaquely (we never parse it; webrtc-rs does).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp_mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mline_index: Option<u16>,
}

/// Minimal signaling envelope, per the locked decision.
/// `kind` discriminates offer/answer; `ice` accumulates trickled candidates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalMessage {
    /// The SDP string for an offer/answer (empty for pure ICE updates).
    pub sdp: String,
    /// `offer` or `answer`.
    pub kind: SignalKind,
    /// Robot id of the sender.
    pub from: String,
    /// Robot id of the intended receiver.
    pub to: String,
    /// Trickled ICE candidates (often empty on the first offer/answer).
    #[serde(default)]
    pub ice: Vec<IceCandidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalKind {
    Offer,
    Answer,
}

/// Presence advertisement: which camera streams this robot offers to peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub id: String,
    /// Key-exprs of camera streams this robot can publish, e.g. `robot/7/local/cam0`.
    #[serde(default)]
    pub streams: Vec<String>,
}

fn replace(template: &str, self_id: &str, peer_id: &str) -> String {
    template
        .replace("{self}", self_id)
        .replace("{peer}", peer_id)
}

/// Publish this robot's presence so peers can discover it and learn its streams.
pub async fn publish_presence(
    transport: &Transport,
    robot_id: &str,
    streams: Vec<String>,
) -> zenoh::Result<()> {
    let key = crate::transport::SIGNAL_PRESENCE_KEY.replace("{id}", robot_id);
    let presence = Presence {
        id: robot_id.to_string(),
        streams,
    };
    let value = serde_json::to_value(&presence).map_err(|e| Box::new(e) as zenoh::Error)?;
    transport.publish_signal(&key, &value).await?;
    info!(robot_id, "published presence");
    Ok(())
}

/// Publish an offer from `robot_id` to `peer_id`.
pub async fn publish_offer(
    transport: &Transport,
    robot_id: &str,
    peer_id: &str,
    sdp: String,
    ice: Vec<IceCandidate>,
) -> zenoh::Result<()> {
    let key = replace(crate::transport::SIGNAL_OFFER_KEY, robot_id, peer_id);
    put_signal(
        transport,
        &key,
        SignalKind::Offer,
        robot_id,
        peer_id,
        sdp,
        ice,
    )
    .await
}

/// Publish an answer from `robot_id` to `peer_id`.
pub async fn publish_answer(
    transport: &Transport,
    robot_id: &str,
    peer_id: &str,
    sdp: String,
    ice: Vec<IceCandidate>,
) -> zenoh::Result<()> {
    let key = replace(crate::transport::SIGNAL_ANSWER_KEY, robot_id, peer_id);
    put_signal(
        transport,
        &key,
        SignalKind::Answer,
        robot_id,
        peer_id,
        sdp,
        ice,
    )
    .await
}

/// Publish a trickled ICE candidate from `robot_id` to `peer_id`.
pub async fn publish_ice(
    transport: &Transport,
    robot_id: &str,
    peer_id: &str,
    ice: IceCandidate,
) -> zenoh::Result<()> {
    let key = replace(crate::transport::SIGNAL_ICE_KEY, robot_id, peer_id);
    put_signal(
        transport,
        &key,
        SignalKind::Offer, // kind is irrelevant for ICE; retained for envelope shape
        robot_id,
        peer_id,
        String::new(),
        vec![ice],
    )
    .await
}

async fn put_signal(
    transport: &Transport,
    key: &str,
    kind: SignalKind,
    from: &str,
    to: &str,
    sdp: String,
    ice: Vec<IceCandidate>,
) -> zenoh::Result<()> {
    let msg = SignalMessage {
        sdp,
        kind,
        from: from.to_string(),
        to: to.to_string(),
        ice,
    };
    let value = serde_json::to_value(&msg).map_err(|e| Box::new(e) as zenoh::Error)?;
    transport.publish_signal(key, &value).await
}

/// Callbacks invoked when inbound signaling messages arrive for this robot.
pub trait SignalHandler {
    /// An offer arrived from `from` for us; the handler should build an answer.
    fn on_offer(&self, from: &str, msg: &SignalMessage);
    /// An answer arrived from `from`.
    fn on_answer(&self, from: &str, msg: &SignalMessage);
    /// A trickled ICE candidate arrived from `from`.
    fn on_ice(&self, from: &str, candidate: &IceCandidate);
}

/// Subscribe to all inbound signal key-exprs addressed to `robot_id` and dispatch
/// to `handler`. Presence is subscribed separately via [`subscribe_presence`].
///
/// Returns an error if any subscription fails; subscriptions otherwise live until
/// the session closes (zenoh owns them after `background()`).
pub async fn run_signal_receiver<H>(
    transport: &Transport,
    robot_id: &str,
    handler: H,
) -> zenoh::Result<()>
where
    H: SignalHandler + Send + Sync + 'static,
{
    let handler = std::sync::Arc::new(handler);
    let self_id = robot_id.to_string();

    // Offers addressed to us: robot/*/signal/<us>/offer
    let offers = format!("robot/*/signal/{}/offer", self_id);
    let h = handler.clone();
    transport
        .subscribe(&offers, move |sample: zenoh::sample::Sample| {
            if let Some(msg) = parse_signal(&sample) {
                h.on_offer(&msg.from, &msg);
            }
        })
        .await?;

    // Answers addressed to us.
    let answers = format!("robot/*/signal/{}/answer", self_id);
    let h = handler.clone();
    transport
        .subscribe(&answers, move |sample: zenoh::sample::Sample| {
            if let Some(msg) = parse_signal(&sample) {
                h.on_answer(&msg.from, &msg);
            }
        })
        .await?;

    // ICE addressed to us.
    let ice = format!("robot/*/signal/{}/ice", self_id);
    let h = handler.clone();
    transport
        .subscribe(&ice, move |sample: zenoh::sample::Sample| {
            if let Some(msg) = parse_signal(&sample) {
                for candidate in &msg.ice {
                    h.on_ice(&msg.from, candidate);
                }
            }
        })
        .await?;

    info!(robot_id, "signal receiver subscribed (offer/answer/ice)");
    Ok(())
}

/// Subscribe to presence advertisements; `on_peer` fires for each discovered peer.
pub async fn subscribe_presence<H>(transport: &Transport, handler: H) -> zenoh::Result<()>
where
    H: Fn(Presence) + Send + Sync + 'static,
{
    let key = crate::transport::SIGNAL_PRESENCE_KEY.replace("{id}", "*");
    transport
        .subscribe(&key, move |sample: zenoh::sample::Sample| {
            let bytes = sample.payload().to_bytes();
            match serde_json::from_slice::<Presence>(&bytes) {
                Ok(p) => handler(p),
                Err(e) => debug!(error = %e, "ignored malformed presence"),
            }
        })
        .await
}

fn parse_signal(sample: &zenoh::sample::Sample) -> Option<SignalMessage> {
    let bytes = sample.payload().to_bytes();
    match serde_json::from_slice::<SignalMessage>(&bytes) {
        Ok(m) => Some(m),
        Err(e) => {
            error!(error = %e, "ignored malformed signal message");
            None
        }
    }
}
