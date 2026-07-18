use zenoh::qos::{CongestionControl, Priority, Reliability};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = zenoh::open(zenoh::Config::default()).await?;

    let stop_pub = session
        .declare_publisher("stop/fleet/cmd")
        .reliability(Reliability::Reliable)
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await?;

    let lidar_pub = session
        .declare_publisher("lidar/fleet/scan")
        .reliability(Reliability::BestEffort)
        .congestion_control(CongestionControl::Drop)
        .priority(Priority::DataLow)
        .await?;

    stop_pub.put([]).await?;
    lidar_pub.put([]).await?;

    let _api = webrtc::api::APIBuilder::new().build();
    Ok(())
}
