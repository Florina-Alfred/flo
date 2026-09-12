#![forbid(unsafe_code)]

use flo_rs::auth::{AuthConfig, AuthMode};
use flo_rs::cli;
use flo_rs::cli::Command;
use flo_rs::health::init_tracing;
use flo_rs::transport::Transport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = cli::parse_server_args();

    if let Some(Command::Rule { command }) = args.command.as_ref() {
        return command.run();
    }

    if args.healthcheck {
        let addr =
            std::env::var("FLO_HEALTH_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        return if flo_rs::health::probe(&addr) {
            Ok(())
        } else {
            Err(format!("health probe failed at {addr}").into())
        };
    }

    if args.print_zenoh_port {
        let robot_id = args
            .robot_id
            .clone()
            .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
            .unwrap_or_else(|| "7".to_string());
        let auth_mode = AuthMode::parse(&args.auth_mode)
            .map_err(|e| format!("invalid --auth-mode '{}': {e}", args.auth_mode))?;
        let auth = AuthConfig {
            mode: auth_mode,
            allow_insecure: args.auth_allow_insecure,
            cert: args.auth_cert.clone().map(std::path::PathBuf::from),
            key: args.auth_key.clone().map(std::path::PathBuf::from),
            trust: args.auth_trust.clone().map(std::path::PathBuf::from),
        };
        auth.validate_production()?;
        let config = auth.zenoh_config(&robot_id)?;
        let transport = Transport::open_with(config).await?;
        let locators = transport.locators().await;
        for loc in &locators {
            println!("{loc}");
        }
        if locators.is_empty() {
            eprintln!("no locators");
            return Err("no zenoh locators".into());
        }
        return Ok(());
    }

    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    flo_rs::server::run_server(args, robot_id).await?;
    Ok(())
}
