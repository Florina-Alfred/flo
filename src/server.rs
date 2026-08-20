use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tracing::info;

use crate::auth::{AuthConfig, AuthMode};
use crate::config::{RuleStore, ServerConfig, run_hot_reload_with_registry};
use crate::engine;
use crate::health::Health;
use crate::registration::{RegistrationServer, run_heartbeat_monitor, run_registration_handler};
use crate::registry::Registry;
use crate::transport::Transport;

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

    // Load server config from environment or default.
    let server_config = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read server config {path}: {e}"))?;
            ServerConfig::from_toml(&text)?
        }
        None => ServerConfig::default(),
    };

    let transport = Arc::new(Transport::open_with(config).await?);
    let store = RuleStore::bootstrap_demo(&robot_id);
    let counter = Arc::new(AtomicU64::new(0));

    let db_path = std::env::temp_dir()
        .join("flo-server-registry")
        .join("audit.db");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let registry = Arc::new(Registry::new(&db_path)?);

    let reg_server = RegistrationServer::new(server_config);

    info!("flo-engine server mode started (robot_id={robot_id})");

    let health = Health::new();
    let health_task = {
        let health = health.clone();
        tokio::spawn(async move {
            let addr = std::env::var("FLO_HEALTH_ADDR").unwrap_or_else(|_| "0.0.0.0:0".to_string());
            if let Err(e) = crate::health::serve(health, &addr).await {
                tracing::error!(error = %e, "health server exited");
            }
        })
    };
    health.set_ready();

    tokio::try_join!(
        engine::run_engine(transport.clone(), store.clone(), counter),
        run_hot_reload_with_registry(&transport, &robot_id, store.clone(), registry),
        run_registration_handler(transport.clone(), reg_server.clone()),
        run_heartbeat_monitor(transport.clone(), reg_server),
        async {
            health_task.await.ok();
            Ok(())
        },
    )?;
    Ok(())
}
