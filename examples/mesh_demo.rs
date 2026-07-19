//! Two-node loopback mesh + rule firing.
//! Run:  cargo run --example mesh_demo
//! Then in another terminal:  cargo run --example mesh_demo -- --robot-id 8
//! Watch for "rule fired" as the two nodes mesh over loopback Zenoh.

use std::sync::Arc;

use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::simulate;
use flo_rs::transport::Transport;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Mirror the `flo` binary: accept --robot-id <id> or FLO_ROBOT_ID.
    let robot_id = std::env::var("FLO_ROBOT_ID").ok().or_else(|| {
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            if a == "--robot-id" {
                if let Some(v) = args.next() {
                    return Some(v);
                }
            }
        }
        None
    });
    let robot_id = robot_id.unwrap_or_else(|| "7".to_string());

    let mut transport = Transport::open_with(Transport::loopback_config())
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    transport
        .declare_liveliness(&robot_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let transport = Arc::new(transport);
    println!("mesh_demo: robot {robot_id} on loopback zenoh");

    let store = RuleStore::bootstrap_demo(&robot_id);
    {
        let t = transport.clone();
        let s = store.clone();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(t, s).await {
                eprintln!("engine exited: {e}");
            }
        });
    }
    {
        let t = transport.clone();
        let r = robot_id.clone();
        tokio::spawn(async move {
            if let Err(e) = simulate::run_simulate(&t, &r, 1000).await {
                eprintln!("simulator exited: {e}");
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    Ok(())
}
