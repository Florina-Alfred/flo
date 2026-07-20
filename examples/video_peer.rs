//! Stream WebRTC video peer-to-peer to a peer id, signaling over Zenoh.
//! Requires the `media` feature + GStreamer:
//!   cargo run --features media --example video_peer -- <peer-id>
//! Without `media` this example refuses to build (clear error below).

#[cfg(feature = "media")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use flo_rs::transport::Transport;
    use flo_rs::video;

    let peer = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: video_peer <peer-id>"))?;
    let robot_id = "7".to_string();

    let mut transport = Transport::open_with(Transport::loopback_config())
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    transport
        .declare_liveliness(&robot_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let transport = std::sync::Arc::new(transport);

    video::start_video_with_source(
        &robot_id,
        &peer,
        transport,
        flo_rs::media::SourceSpec::Videotest,
    )
    .await
    .map_err(|e| anyhow::anyhow!("video failed: {e}"))?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(not(feature = "media"))]
fn main() {
    eprintln!(
        "video_peer requires the `media` feature: cargo run --features media --example video_peer -- <peer-id>"
    );
    std::process::exit(1);
}
