#![forbid(unsafe_code)]

use flo_rs::cli;
use flo_rs::health::init_tracing;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = cli::parse_args();

    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    flo_rs::server::run_server(args, robot_id).await?;
    Ok(())
}
