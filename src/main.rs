//! `flo` binary entry point.
//!
//! Thin orchestration only: parse CLI, pick a mode (demo / production / rule
//! subcommand / media self-test), and delegate to the dedicated modules. All
//! reusable runtime wiring lives in `common`, `demo`, `production`, `mesh`.

#![forbid(unsafe_code)]

mod cli;
mod codec;
mod common;
mod config;
mod demo;
mod device;
mod engine;
mod health;
mod mesh;
mod production;
mod rules;
mod signaling;
mod simulate;
mod transport;
mod video;

#[cfg(feature = "media")]
mod media;

use cli::Command;
use common::run_rule_command;
use health::init_tracing;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = cli::parse_args();

    // `flo rule ...` exits the process itself (validation tooling).
    if let Some(Command::Rule { args: rule_args }) = args.command {
        return run_rule_command(&rule_args);
    }

    // Demo mode = no explicit production flags. `cargo run` with no args lands here.
    let demo = args.robot_id.is_none() && args.config.is_none();
    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    #[cfg(feature = "media")]
    if args.video.self_test {
        return run_video_self_test(&args.video.codec);
    }

    if demo {
        demo::run_demo(args, robot_id).await?;
    } else {
        production::run_production(args, robot_id).await?;
    }
    Ok(())
}

/// Encode-only self-test (media feature): verify GStreamer produces H.264.
#[cfg(feature = "media")]
fn run_video_self_test(_codec: &codec::Codec) -> anyhow::Result<()> {
    use crate::media::{MediaPipeline, SourceSpec};
    let pipeline = MediaPipeline::build(&SourceSpec::Videotest, 1280, 720, 30)?;
    let found = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = found.clone();
    pipeline.start(Box::new(move |bytes: &[u8]| {
        // Annex-B start code: 00 00 00 01
        if bytes.windows(4).any(|w| w == [0x00, 0x00, 0x00, 0x01]) {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::info!(
                len = bytes.len(),
                "▶ encoded H.264 sample (Annex-B start code ok)"
            );
        }
    }))?;
    // Run a few seconds to pull samples.
    std::thread::sleep(std::time::Duration::from_secs(3));
    pipeline.stop();
    anyhow::ensure!(
        found.load(std::sync::atomic::Ordering::SeqCst),
        "no encoded H.264 samples produced"
    );
    println!("SELF-TEST OK: gstreamer encode produced H.264");
    Ok(())
}
