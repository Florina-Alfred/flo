#![forbid(unsafe_code)]

use flo_rs::cli;
use flo_rs::cli::Command;
use flo_rs::common::run_rule_command;
use flo_rs::health::init_tracing;
use flo_rs::runtime::ClientRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = cli::parse_args();

    if let Some(Command::Rule { command }) = args.command.as_ref() {
        return run_rule_command(command);
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

    ClientRuntime::run(args).await
}
