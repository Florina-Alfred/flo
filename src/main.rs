#![forbid(unsafe_code)]

mod config;
mod engine;
mod health;
mod rules;
mod signaling;
mod transport;

use std::sync::Arc;

use tracing::{error, info};

use crate::config::{run_hot_reload, RuleStore};
use crate::health::Health;
use crate::transport::Transport;

const BOOTSTRAP_RULES_PATH: &str = "/etc/flo/rules.toml";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    health::init_tracing();

    let robot_id = std::env::var("FLO_ROBOT_ID").unwrap_or_else(|_| "0".to_string());
    info!(robot_id, "starting flo client");

    let bootstrap = std::fs::read_to_string(BOOTSTRAP_RULES_PATH)
        .unwrap_or_else(|_| "rules = []\n".to_string());
    let store = RuleStore::bootstrap(&bootstrap)
        .map_err(|e| format!("invalid bootstrap rules at {BOOTSTRAP_RULES_PATH}: {e}"))?;

    let mut transport = Transport::open().await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "zenoh session open, liveliness declared");

    let health = Health::new();

    // Health server (map 04): HTTP probes, not exec.
    let health_task = {
        let health = health.clone();
        tokio::spawn(async move {
            if let Err(e) = health::serve(health, "0.0.0.0:8080").await {
                error!(error = %e, "health server exited");
            }
        })
    };

    // Hot-reload subscriber (map 02): zenoh topic swaps the ruleset atomically.
    let reload_task = {
        let transport = transport.clone();
        let store = store.clone();
        let robot_id = robot_id.clone();
        tokio::spawn(async move {
            if let Err(e) = run_hot_reload(&transport, &robot_id, store).await {
                error!(error = %e, "hot-reload subscriber exited");
            }
        })
    };

    // Rule engine eval loop (map 02): sensors -> composable rules -> actuator actions.
    let engine_task = {
        let transport = transport.clone();
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(transport, store).await {
                error!(error = %e, "rule engine exited");
            }
        })
    };

    // WebRTC signaling (webrtc-signaling map): advertise presence + receive offers/
    // answers/ICE over the zenoh mesh. Live peer connections are later work; the
    // handler below logs inbound signals until then.
    let signal_task = {
        let transport = transport.clone();
        let robot_id = robot_id.clone();
        tokio::spawn(async move {
            if let Err(e) = run_signaling(&transport, &robot_id).await {
                error!(error = %e, "signaling exited");
            }
        })
    };

    health.set_ready();
    info!("flo client ready");

    // Run until any subsystem dies (k8s will restart the pod).
    tokio::select! {
        r = health_task => error!(?r, "health task ended"),
        r = reload_task => error!(?r, "reload task ended"),
        r = engine_task => error!(?r, "engine task ended"),
        r = signal_task => error!(?r, "signaling task ended"),
    }
    Ok(())
}

/// Start the WebRTC signaling plane: publish presence and receive inbound signals.
/// The handler is a placeholder that logs; attaching webrtc-rs peer connections is
/// future work (see the webrtc-signaling map's out-of-scope browser/video UI).
async fn run_signaling(transport: &Transport, robot_id: &str) -> zenoh::Result<()> {
    signaling::publish_presence(transport, robot_id, vec![format!("robot/{}/local/cam0", robot_id)]).await?;

    let handler = LoggingSignalHandler;
    // Peer discovery: log any robot that advertises presence.
    signaling::subscribe_presence(transport, |p: signaling::Presence| {
        info!(peer = %p.id, streams = ?p.streams, "discovered peer");
    })
    .await?;

    signaling::run_signal_receiver(transport, robot_id, handler).await
}

/// Placeholder signal handler: logs inbound offers/answers/ICE. Replaced by a
/// webrtc-rs peer-connection driver when class-3 media is implemented.
struct LoggingSignalHandler;

impl signaling::SignalHandler for LoggingSignalHandler {
    fn on_offer(&self, from: &str, msg: &signaling::SignalMessage) {
        info!(from, sdp_len = msg.sdp.len(), ice = msg.ice.len(), "received offer (no media yet)");
    }
    fn on_answer(&self, from: &str, msg: &signaling::SignalMessage) {
        info!(from, sdp_len = msg.sdp.len(), "received answer (no media yet)");
    }
    fn on_ice(&self, from: &str, candidate: &signaling::IceCandidate) {
        info!(from, candidate = %candidate.candidate, "received ICE candidate (no media yet)");
    }
}
