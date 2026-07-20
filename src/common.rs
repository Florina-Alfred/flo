//! Shared runtime wiring used by both demo and production modes, plus the
//! `rule` subcommand. Keeping these here lets `main.rs` stay a thin entry point
//! and lets `demo`/`production` modules focus on mode-specific input + rules.

use std::sync::Arc;

use tracing::{error, info};

use crate::cli::Args;
use crate::config::{RuleStore, run_hot_reload};
use crate::engine;
use crate::health;
use crate::health::Health;
use crate::mesh::run_signaling;
use crate::transport::Transport;

/// Start health server, hot-reload, rule engine, and WebRTC signaling. Shared by
/// both demo and production modes (the only difference is input + rules source).
pub async fn start_common_subsystems(
    transport: &Arc<Transport>,
    store: &RuleStore,
    robot_id: &str,
) {
    let health = Health::new();

    let health_task = {
        let health = health.clone();
        tokio::spawn(async move {
            if let Err(e) = health::serve(health, "0.0.0.0:8080").await {
                error!(error = %e, "health server exited");
            }
        })
    };

    let reload_task = {
        let transport = transport.clone();
        let store = store.clone();
        let robot_id = robot_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = run_hot_reload(&transport, &robot_id, store).await {
                error!(error = %e, "hot-reload subscriber exited");
            }
        })
    };

    let engine_task = {
        let transport = transport.clone();
        let store = store.clone();
        let eval_counter = health.eval_counter();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(transport, store, eval_counter).await {
                error!(error = %e, "rule engine exited");
            }
        })
    };

    let signal_task = {
        let transport = transport.clone();
        let robot_id = robot_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = run_signaling(transport.clone(), &robot_id).await {
                error!(error = %e, "signaling exited");
            }
        })
    };

    health.set_ready();
    info!("flo ready");

    // Store handles so they live for the process; tasks are joined by the caller.
    let _ = (health_task, reload_task, engine_task, signal_task);
}

/// Run until any subsystem dies (k8s / process supervisor restarts).
pub async fn wait_for_subsystems() {
    // The spawned tasks own the long-lived work; this future just idles. A real
    // deployment would `tokio::select!` on the JoinHandles. For the demo we block
    // so `cargo run` stays alive and visible.
    std::future::pending::<()>().await;
}

/// Spawn the outbound WebRTC video call requested via `--video-peer`, validating
/// the configured device up front so a bad path fails fast with a clear message
/// instead of an opaque GStreamer error. Shared by demo and production modes.
pub fn spawn_video_peer(args: &Args, transport: Arc<Transport>, robot_id: String) {
    let Some(peer) = args.video.peer.clone() else {
        return;
    };
    let tr = transport.clone();
    let rid = robot_id.clone();
    let pid = peer.clone();
    // Validate the configured video device up front so a bad path fails
    // fast with a clear message instead of an opaque GStreamer error.
    #[cfg_attr(not(feature = "media"), allow(unused_variables))]
    let device = match &args.video.device {
        Some(d) => match crate::device::VideoDevice::validate(d) {
            Ok(dev) => Some(dev),
            Err(e) => {
                tracing::error!(error = %e, "invalid --video-device, falling back to test pattern");
                None
            }
        },
        None => None,
    };
    tokio::spawn(async move {
        #[cfg(feature = "media")]
        {
            use crate::media::SourceSpec;
            let source = match device {
                Some(dev) => dev.to_source_spec(),
                None => SourceSpec::Videotest,
            };
            if let Err(e) = crate::video::start_video_with_source(&rid, &pid, tr, source).await {
                tracing::error!(error = %e, "video failed");
            }
        }
        #[cfg(not(feature = "media"))]
        {
            if let Err(e) = crate::video::start_video(&rid, &pid, tr).await {
                tracing::error!(error = %e, "video failed");
            }
        }
    });
}

/// Handle the `flo rule check <path>` subcommand: validate a semantic ruleset
/// (extended TOML) before deploy. Exits the process on invalid input.
pub fn run_rule_command(cmd: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd.first().map(String::as_str) {
        Some("check") => {
            let path = cmd.get(1).ok_or("usage: flo rule check <path>")?;
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            match flo_rs::semantic::parse_semantic(&text) {
                Ok(doc) => match flo_rs::semantic::validate(&doc) {
                    Ok(()) => {
                        println!("OK: {path} is a valid semantic ruleset");
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("INVALID: {e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("PARSE ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown rule subcommand: {other:?} (try 'flo rule check <path>')");
            std::process::exit(2);
        }
    }
}
