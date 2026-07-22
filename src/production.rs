//! Production mode: file-based rules, optional external config, no simulation
//! unless explicitly requested (e.g. a dev node without hardware).

use std::sync::Arc;

use tracing::{error, info};

use flo_rs::auth::{AuthConfig, AuthMode};
use flo_rs::config::RuleStore;
use flo_rs::semantic;
use flo_rs::simulate;
use flo_rs::transport::Transport;

use crate::cli::Args;
use crate::common::{spawn_video_peer, start_common_subsystems, block_indefinitely};

/// Run in production mode (k8s DaemonSet): load rules from `--config`, open a
/// real zenoh session, and start the shared subsystems.
pub async fn run_production(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(robot_id, "starting flo client (production mode)");

    // Build + validate the auth config before opening any session. Production
    // hard-blocks `auth: none` unless explicitly overridden; this fails fast.
    let auth_mode = AuthMode::parse(&args.auth_mode)
        .map_err(|e| format!("invalid --auth-mode '{0}': {e}", args.auth_mode))?;
    let auth = AuthConfig {
        mode: auth_mode,
        allow_insecure: args.auth_allow_insecure,
        cert: args.auth_cert.clone().map(std::path::PathBuf::from),
        key: args.auth_key.clone().map(std::path::PathBuf::from),
        trust: args.auth_trust.clone().map(std::path::PathBuf::from),
    };
    if auth.mode.is_authenticated() {
        auth.validate_production()
            .map_err(|e| format!("auth config invalid: {e}"))?;
        info!(mode = ?auth.mode, "auth validated (authenticated client)");
    } else {
        match auth.validate_production() {
            Ok(_) => tracing::warn!(
                "auth: none permitted via --auth-allow-insecure; NO impersonation protection"
            ),
            Err(_) => {
                return Err(
                    "auth: none is blocked in production; set --auth-allow-insecure for dev/air-gapped only"
                        .into(),
                )
            }
        }
    }

    let bootstrap = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(path, error = %e, "config unreadable -> starting in fail-safe safe-state (no unrestricted motion)");
                empty_ruleset_toml()
            }
        },
        None => "rules = []\n".to_string(),
    };

    // Try semantic (extended-TOML) first; fall back to raw TOML; else safe-state.
    let store = compile_rules_or_default(&bootstrap, &robot_id);

    let mut transport = Transport::open_with(
        auth.zenoh_config(&robot_id)
            .map_err(|e| format!("auth config invalid: {e}"))?,
    )
    .await?;
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
            if let Err(e) = simulate::simulate_sensors(&transport_sim, &robot_id_sim, period).await {
                error!(error = %e, "simulator exited");
            }
        });
    }

    spawn_video_peer(&args, transport, robot_id);

    block_indefinitely().await;
    Ok(())
}

/// A minimal fail-safe ruleset: no motion commands are emitted.
fn empty_ruleset_toml() -> String {
    "rules = []\n".to_string()
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_rules_or_default(text: &str, robot_id: &str) -> RuleStore {
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
            RuleStore::bootstrap(&empty_ruleset_toml()).expect("empty ruleset always parses")
        }
    }
}
