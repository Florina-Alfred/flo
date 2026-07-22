//! Simulated sensor source for the local demo.
//!
//! Publishes synthetic sensor samples on the zenoh topics the rule engine watches,
//! using the real [`Transport::publish`] path. This is a *publisher* only — the rule
//! engine code is unchanged and consumes these topics exactly as it would real
//! device data. No hardware, no `libudev`, no camera required.

use std::time::Duration;

use serde_json::json;
use tokio::time::interval;
use tracing::info;

use crate::rules::Qos;
use crate::transport::Transport;

/// Default sensor topics the demo rules watch (mirror map-02's example rules).
pub const BUMPER_TOPIC: &str = "robot/{id}/local/bumper";
pub const IMU_TOPIC: &str = "robot/{id}/local/imu";
pub const LIDAR_TOPIC: &str = "lidar/fleet/scan";

/// Run the simulator: every `period` ms, emit one round of synthetic samples.
/// `bumper_pressed` toggles each round so the e-stop rule fires on alternate ticks;
/// `imu_speed` and `lidar_range` dip below their rule thresholds periodically so the
/// lidar-slowdown rule also fires, giving the user a visible "aha" on first run.
pub async fn simulate_sensors(
    transport: &Transport,
    robot_id: &str,
    period_ms: u64,
) -> zenoh::Result<()> {
    let bumper = BUMPER_TOPIC.replace("{id}", robot_id);
    let imu = IMU_TOPIC.replace("{id}", robot_id);
    let lidar = LIDAR_TOPIC.to_string();

    // Default demo tick is 1 round / second so the "aha" is easy to read.
    // For a busier stream, drop the default in `main.rs` (or pass --simulate-period-ms):
    //     let period_ms = 250; // <- commented-out faster tick; uncomment to speed up
    let mut tick = interval(Duration::from_millis(period_ms.max(100)));
    let mut round: u64 = 0;
    info!(robot_id, period_ms, "simulator started (synthetic sensors)");

    loop {
        tick.tick().await;
        round = round.wrapping_add(1);

        // Bumper pressed every other round -> e-stop rule fires when also moving.
        let pressed = round.is_multiple_of(2);
        transport
            .publish(&bumper, Qos::Reliable, &json!({ "pressed": pressed }))
            .await?;

        // IMU speed: the robot keeps moving (0.4 m/s) so that a pressed bumper
        // satisfies the e-stop condition (pressed AND speed > 0.2) and fires.
        let speed = 0.4;
        transport
            .publish(&imu, Qos::Reliable, &json!({ "speed_mps": speed }))
            .await?;

        // Lidar min range dips below 0.5 every 4th round -> slowdown rule fires.
        let min_range = if round.is_multiple_of(4) { 0.3 } else { 1.2 };
        transport
            .publish(
                &lidar,
                Qos::BestEffort,
                &json!({ "min_range_m": min_range }),
            )
            .await?;

        info!(
            round,
            pressed,
            speed_mps = speed,
            min_range_m = min_range,
            "simulated sensor round"
        );
    }
}
