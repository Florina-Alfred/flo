//! Local demo mode: loopback zenoh, built-in rules, loud verdicts.
//! `cargo run` with no args lands here.

use std::sync::Arc;

use tracing::info;

use crate::auth::{AuthConfig, AuthMode};
use crate::config::RuleStore;
use crate::transport::Transport;

use crate::cli::Args;
use crate::common::{block_indefinitely, spawn_video_peer, start_common_subsystems};

/// Run the local demo: rule engine on a loopback zenoh mesh.
pub async fn run_demo(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Demo is a dev/loopback deployment: validate auth in dev mode so a stray
    // `auth: none` is accepted but authenticated configs are still checked.
    let auth_mode = AuthMode::parse(&args.auth_mode)
        .map_err(|e| format!("invalid --auth-mode '{0}': {e}", args.auth_mode))?;
    let auth = AuthConfig {
        mode: auth_mode,
        allow_insecure: args.auth_allow_insecure,
        cert: args.auth_cert.clone().map(std::path::PathBuf::from),
        key: args.auth_key.clone().map(std::path::PathBuf::from),
        trust: args.auth_trust.clone().map(std::path::PathBuf::from),
    };
    auth.validate_dev()
        .map_err(|e| format!("auth config invalid (dev): {e}"))?;

    println!(
        "\n  flo DEMO  —  robot {robot_id} on loopback zenoh\n\
         \x20\x20Simulating sensors and running the rule engine. Watch for '▶ rule fired'.\n\
         \x20\x20Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)\n"
    );

    let mut transport = Transport::open_with(Transport::loopback_config()).await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "demo zenoh session open (loopback peer mesh)");

    let store = RuleStore::bootstrap_demo(&robot_id);

    start_common_subsystems(&transport, &store, &robot_id, &args).await;

    spawn_video_peer(&args, transport, robot_id);

    block_indefinitely().await;
    Ok(())
}
