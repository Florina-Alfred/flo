//! Shared subsystem wiring for the client runtime, plus the `rule` subcommand.
//! Keeping these here lets `runtime.rs` stay focused on client orchestration.

use std::sync::Arc;

use tracing::{error, info};

use crate::config::{RuleStore, run_hot_reload};
use crate::engine;
use crate::transport::Transport;

use crate::cli::Args;
use crate::health;
use crate::health::Health;
#[cfg(feature = "media")]
use crate::mesh::run_signaling;

/// Handles to the spawned subsystems, for supervision by the client runtime.
#[derive(Debug)]
pub struct SubsystemHandles {
    /// HTTP health/liveness server.
    pub health: tokio::task::JoinHandle<()>,
    /// Ruleset hot-reload subscriber.
    pub reload: tokio::task::JoinHandle<()>,
    /// Rule engine.
    pub engine: tokio::task::JoinHandle<()>,
    /// WebRTC signaling (always-on answerer / peer discovery).
    pub signaling: tokio::task::JoinHandle<()>,
}

/// Start the health server, hot-reload, rule engine, and WebRTC signaling and
/// return their handles for supervision.
///
/// `args` is used (under the `media` feature) to resolve the configured capture
/// device so the always-on answerer can stream video back when a device is set.
pub async fn start_common_subsystems(
    transport: &Arc<Transport>,
    store: &RuleStore,
    robot_id: &str,
    #[cfg_attr(not(feature = "media"), allow(unused_variables))] args: &Args,
) -> SubsystemHandles {
    let health = Health::new();

    let health_task = {
        let health = health.clone();
        tokio::spawn(async move {
            let addr = std::env::var("FLO_HEALTH_ADDR").unwrap_or_else(|_| "0.0.0.0:0".to_string());
            if let Err(e) = health::serve(health, &addr).await {
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
        #[cfg(feature = "media")]
        let transport = transport.clone();
        #[cfg(feature = "media")]
        let robot_id = robot_id.to_string();
        #[cfg(feature = "media")]
        let source = match &args.video.device {
            Some(d) => crate::device::VideoDevice::from_path(d)
                .ok()
                .map(|dev| dev.to_source_spec()),
            None => None,
        };
        #[cfg(feature = "media")]
        {
            tokio::spawn(async move {
                if let Err(e) = run_signaling(transport.clone(), &robot_id, source).await {
                    error!(error = %e, "signaling exited");
                }
            })
        }
        #[cfg(not(feature = "media"))]
        {
            tokio::spawn(async move {
                // Signaling requires the `media` feature (WebRTC/mesh listener).
                // Without it, no peer-discovery or inbound video-answer is started.
                std::future::pending::<()>().await
            })
        }
    };

    health.set_ready();
    info!("flo ready");

    SubsystemHandles {
        health: health_task,
        reload: reload_task,
        engine: engine_task,
        signaling: signal_task,
    }
}

/// Spawn the outbound WebRTC video call requested via `--video-peer`, validating
/// the configured device up front so a bad path fails fast with a clear message
/// instead of an opaque GStreamer error.
/// Only available with the `media` feature (requires system GStreamer + webrtc).
#[cfg(feature = "media")]
pub fn spawn_video_peer(args: &Args, transport: Arc<Transport>, robot_id: String) {
    let Some(peer) = args.video.peer.clone() else {
        return;
    };
    let tr = transport.clone();
    let rid = robot_id.clone();
    let pid = peer.clone();
    // Validate the configured video device up front so a bad path fails
    // fast with a clear message instead of an opaque GStreamer error.
    let device = match &args.video.device {
        Some(d) => match crate::device::VideoDevice::from_path(d) {
            Ok(dev) => Some(dev),
            Err(e) => {
                tracing::error!(error = %e, "invalid --video-device, falling back to test pattern");
                None
            }
        },
        None => None,
    };
    tokio::spawn(async move {
        use crate::media::SourceSpec;
        let source = match device {
            Some(dev) => dev.to_source_spec(),
            None => SourceSpec::Videotest,
        };
        if let Err(e) = crate::video::start_video_with_source(&rid, &pid, tr, source).await {
            tracing::error!(error = %e, "video failed");
        }
    });
}

/// Stub compiled when `media` feature is off: logs a hint and returns.
#[cfg(not(feature = "media"))]
pub fn spawn_video_peer(_args: &Args, _transport: Arc<Transport>, _robot_id: String) {
    if _args.video.peer.is_some() {
        tracing::info!(
            _robot_id,
            "--video-peer set but media feature disabled; recompile with --features media"
        );
    }
}

/// Handle the `flo rule check <path>` and `flo rule compile <path>` subcommands.
/// Validates / compiles a semantic ruleset (extended TOML) before deploy.
/// Exits the process on invalid input.
pub fn run_rule_command(cmd: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let wants_json = cmd.iter().any(|a| a == "--json");
    let args: Vec<&str> = cmd
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--json")
        .collect();

    match args.first().copied() {
        Some("check") => {
            let path = args.get(1).ok_or("usage: flo rule check <path> [--json]")?;
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            match crate::semantic::parse_semantic_auto(&text) {
                Ok(doc) => match crate::semantic::validate(&doc) {
                    Ok(()) => {
                        if wants_json {
                            println!("{}", serde_json::json!({ "status": "ok", "path": path }));
                        } else {
                            println!("OK: {path} is a valid semantic ruleset");
                        }
                        Ok(())
                    }
                    Err(e) => {
                        if wants_json {
                            eprintln!(
                                "{}",
                                serde_json::json!({ "status": "error", "path": path, "error": e.to_string() })
                            );
                        } else {
                            eprintln!("INVALID: {e}");
                        }
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    if wants_json {
                        eprintln!(
                            "{}",
                            serde_json::json!({ "status": "error", "path": path, "error": e.to_string() })
                        );
                    } else {
                        eprintln!("PARSE ERROR: {e}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Some("compile") => {
            let path = args.get(1).ok_or("usage: flo rule compile <path>")?;
            let robot_id = args.get(2).copied().unwrap_or("7");
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let doc = crate::semantic::parse_semantic_auto(&text).map_err(|e| {
                eprintln!("PARSE ERROR: {e}");
                std::process::exit(1);
            })?;
            let rules = crate::semantic::compile(&doc, robot_id).map_err(|e| {
                eprintln!("COMPILE ERROR: {e}");
                std::process::exit(1);
            })?;
            println!("{}", serde_json::to_string_pretty(&rules).unwrap());
            Ok(())
        }
        other => {
            eprintln!(
                "unknown rule subcommand: {other:?} (try 'flo rule check <path>' or 'flo rule compile <path>')"
            );
            std::process::exit(2);
        }
    }
}
