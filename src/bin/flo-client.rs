#![forbid(unsafe_code)]

use flo_rs::cli;
use flo_rs::cli::Command;
use flo_rs::common::run_rule_command;
use flo_rs::health::init_tracing;

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

    if args.mode == cli::Mode::Server {
        flo_rs::server::run_server(args, robot_id).await?;
    } else {
        flo_rs::production::run_production(args, robot_id).await?;
    }
    Ok(())
}
