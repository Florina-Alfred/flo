use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tracing::info;

use flo_rs::auth::{AuthConfig, AuthMode};
use flo_rs::config::{RuleStore, run_hot_reload_with_registry};
use flo_rs::engine;
use flo_rs::registry::Registry;
use flo_rs::transport::Transport;

use crate::cli;

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

    let db_path = std::env::temp_dir()
        .join("flo-server-registry")
        .join("audit.db");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let registry = Arc::new(Registry::new(&db_path)?);

    info!("flo-engine server mode started (robot_id={robot_id})");

    tokio::try_join!(
        engine::run_engine(transport.clone(), store.clone(), counter),
        run_hot_reload_with_registry(&transport, &robot_id, store.clone(), registry),
    )?;
    Ok(())
}
