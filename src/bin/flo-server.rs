#![forbid(unsafe_code)]

use flo_rs::cli;
use flo_rs::cli::Command;
use flo_rs::health::init_tracing;

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

    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    flo_rs::server::run_server(args, robot_id).await?;
    Ok(())
}
