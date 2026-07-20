//! Production mode: file-based rules, optional external config, no simulation
//! unless explicitly requested (e.g. a dev node without hardware).

use std::sync::Arc;

use tracing::{error, info};

use crate::cli::Args;
use crate::common::{spawn_video_peer, start_common_subsystems, wait_for_subsystems};
use crate::config::RuleStore;
use crate::simulate;
use crate::transport::Transport;
use flo_rs::semantic;

/// Run in production mode (k8s DaemonSet): load rules from `--config`, open a
/// real zenoh session, and start the shared subsystems.
pub async fn run_production(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(robot_id, "starting flo client (production mode)");

    let bootstrap = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(path, error = %e, "config unreadable -> starting in fail-safe safe-state (no unrestricted motion)");
                safe_state_toml()
            }
        },
        None => "rules = []\n".to_string(),
    };

    // Try semantic (extended-TOML) first; fall back to raw TOML; else safe-state.
    let store = compile_or_fallback(&bootstrap, &robot_id);

    let mut transport = Transport::open().await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "zenoh session open, liveliness declared");

    start_common_subsystems(&transport, &store, &robot_id, &args).await;

    // Optional simulation in production (e.g. a dev node without hardware).
    if args.simulate {
        let transport_sim = transport.clone();
        let robot_id_sim = robot_id.clone();
        let period = args.simulate_period_ms.max(100);
        tokio::spawn(async move {
            if let Err(e) = simulate::run_simulate(&transport_sim, &robot_id_sim, period).await {
                error!(error = %e, "simulator exited");
            }
        });
    }

    spawn_video_peer(&args, transport, robot_id);

    wait_for_subsystems().await;
    Ok(())
}

/// A minimal fail-safe ruleset: no motion commands are emitted.
fn safe_state_toml() -> String {
    "rules = []\n".to_string()
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_or_fallback(text: &str, robot_id: &str) -> RuleStore {
    if let Ok(doc) = semantic::parse_semantic(text) {
        match semantic::compile(&doc, robot_id) {
            Ok(rules) => match RuleStore::bootstrap(&rules.to_toml()) {
                Ok(s) => return s,
                Err(e) => {
                    tracing::error!(error = %e, "semantic compile produced invalid rules -> safe-state")
                }
            },
            Err(e) => tracing::error!(error = %e, "semantic validation failed -> safe-state"),
        }
    }
    match RuleStore::bootstrap(text) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "config invalid -> starting in fail-safe safe-state");
            RuleStore::bootstrap(&safe_state_toml()).expect("safe-state always parses")
        }
    }
}
