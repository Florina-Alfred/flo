#![forbid(unsafe_code)]

use std::sync::Arc;

use tracing::info;

use flo_rs::cli;
use flo_rs::cli::Command;
use flo_rs::common::{block_indefinitely, run_rule_command, start_common_subsystems};
use flo_rs::config::{ClientConfig, RuleStore};
use flo_rs::health::init_tracing;
use flo_rs::mutation::compute_sha;
use flo_rs::registration::{RegistrationError, register_with_client};
use flo_rs::rules::Rules;
use flo_rs::transport::Transport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = cli::parse_args();

    if let Some(Command::Rule { args: rule_args }) = args.command {
        return run_rule_command(&rule_args);
    }

    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    // Load client config.
    let client_config = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read client config {path}: {e}"))?;
            ClientConfig::from_toml(&text)?
        }
        None => {
            return Err("client config required (use --config <path>)".into());
        }
    };

    // Load optional ruleset file; compute its SHA for dedup / audit.
    let store = if let Some(path) = &args.ruleset {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read ruleset {path}: {e}"))?;
        let sha = compute_sha(raw.as_bytes());
        let rules = Rules::from_toml(&raw)?;
        info!(%robot_id, %sha, "ruleset loaded");
        RuleStore::new(Arc::new(rules))
    } else {
        info!("no ruleset file — using built-in demo");
        RuleStore::bootstrap_demo(&robot_id)
    };

    // Open Zenoh session.
    let mut transport = Transport::open_with(Transport::loopback_config()).await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);

    // Register with the server.
    info!(%robot_id, "registering with server...");
    match register_with_client(&transport, &robot_id, &client_config).await {
        Ok(()) => info!("registration confirmed"),
        Err(RegistrationError::AlreadyRegistered) => {
            return Err("client already registered with server".into());
        }
        Err(RegistrationError::Poisoned) => {
            return Err("client is poisoned on server — cannot join".into());
        }
        Err(RegistrationError::Timeout) => {
            return Err("registration timed out after 3 retries".into());
        }
        Err(RegistrationError::ServerError(e)) => {
            return Err(format!("registration rejected: {e}").into());
        }
    }

    start_common_subsystems(&transport, &store, &robot_id, &args).await;

    block_indefinitely().await;
    Ok(())
}
