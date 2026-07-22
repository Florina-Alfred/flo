use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tracing::info;

use crate::auth::{AuthConfig, AuthMode};
use crate::cli;
use crate::config::RuleStore;
use crate::engine;
use crate::transport::Transport;

pub async fn run_server(
    args: cli::Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let auth_mode = AuthMode::parse(&args.auth_mode)
        .map_err(|e| format!("invalid --auth-mode '{0}': {e}", args.auth_mode))?;
    let auth = AuthConfig {
        mode: auth_mode,
        allow_insecure: args.auth_allow_insecure,
        cert: args.auth_cert.clone().map(std::path::PathBuf::from),
        key: args.auth_key.clone().map(std::path::PathBuf::from),
        trust: args.auth_trust.clone().map(std::path::PathBuf::from),
    };
    auth.validate_production()?;
    let config = auth.zenoh_config(&robot_id)?;
    let session = zenoh::open(config).await?;
    let transport = Arc::new(Transport::from_session(session));
    let store = RuleStore::bootstrap_demo(&robot_id);
    let counter = Arc::new(AtomicU64::new(0));

    info!("flo-engine server mode started (robot_id={robot_id})");
    engine::run_engine(transport, store, counter).await?;
    Ok(())
}
