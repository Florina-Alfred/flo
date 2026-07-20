//! Local demo mode: loopback zenoh, built-in rules, simulated sensors, loud
//! verdicts. `cargo run` with no args lands here.

use std::sync::Arc;

use tracing::{error, info};

use crate::cli::Args;
use crate::common::{spawn_video_peer, start_common_subsystems, wait_for_subsystems};
use crate::config::RuleStore;
use crate::simulate;
use crate::transport::Transport;

/// Run the local demo: simulated sensors + rule engine on a loopback zenoh mesh.
pub async fn run_demo(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "\n  flo DEMO  —  robot {robot_id} on loopback zenoh\n\
         \x20\x20Simulating sensors and running the rule engine. Watch for '▶ rule fired'.\n\
         \x20\x20Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)\n"
    );

    let mut transport = Transport::open_with(Transport::loopback_config()).await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "demo zenoh session open (loopback peer mesh)");

    let store = RuleStore::bootstrap_demo(&robot_id);

    start_common_subsystems(&transport, &store, &robot_id).await;

    // Simulated sensor input (the demo's fake hardware). Demo always simulates.
    let transport_sim = transport.clone();
    let robot_id_sim = robot_id.clone();
    let period = args.simulate_period_ms.max(100);
    tokio::spawn(async move {
        if let Err(e) = simulate::run_simulate(&transport_sim, &robot_id_sim, period).await {
            error!(error = %e, "simulator exited");
        }
    });

    spawn_video_peer(&args, transport, robot_id);

    wait_for_subsystems().await;
    Ok(())
}
