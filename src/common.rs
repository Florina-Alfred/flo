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
    /// WebRTC signaling (always-on answerer / peer discovery), media feature only.
    #[cfg(feature = "media")]
    pub signaling: tokio::task::JoinHandle<()>,
}

/// Start the health server, hot-reload, rule engine, and WebRTC signaling and
/// return their handles for supervision.
///
/// Readiness is gated on the rule engine confirming its subscriptions: `/readyz`
/// flips 200 only after the engine's initial sensor topics are live, so the probe
/// never reports ready while the engine is still subscribing (or dead).
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

    // The engine signals on this channel once its initial subscriptions are live;
    // dropping it without a send (engine died first) keeps readiness un-set.
    let (subscribed_tx, subscribed_rx) = tokio::sync::oneshot::channel::<()>();

    let engine_task = {
        let transport = transport.clone();
        let store = store.clone();
        let eval_counter = health.eval_counter();
        tokio::spawn(async move {
            if let Err(e) =
                engine::run_engine(transport, store, eval_counter, Some(subscribed_tx)).await
            {
                error!(error = %e, "rule engine exited");
            }
        })
    };

    #[cfg(feature = "media")]
    let signal_task = {
        let transport = transport.clone();
        let robot_id = robot_id.to_string();
        let source = match &args.video.device {
            Some(d) => crate::device::VideoDevice::from_path(d)
                .ok()
                .map(|dev| dev.to_source_spec()),
            None => None,
        };
        tokio::spawn(async move {
            if let Err(e) = run_signaling(transport.clone(), &robot_id, source).await {
                error!(error = %e, "signaling exited");
            }
        })
    };

    await_engine_ready(subscribed_rx, &health).await;

    SubsystemHandles {
        health: health_task,
        reload: reload_task,
        engine: engine_task,
        #[cfg(feature = "media")]
        signaling: signal_task,
    }
}

/// Gate readiness on the engine confirming its subscriptions. When the engine
/// dies before confirming (its sender is dropped), readiness stays un-set and
/// the client's supervision observes the dead engine and exits non-zero.
async fn await_engine_ready(
    subscribed: tokio::sync::oneshot::Receiver<()>,
    health: &Health,
) -> bool {
    match subscribed.await {
        Ok(()) => {
            health.set_ready();
            info!("flo ready");
            true
        }
        Err(_) => {
            error!("rule engine died before confirming subscriptions; /readyz stays not-ready");
            false
        }
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
pub fn run_rule_command(
    cmd: &crate::cli::RuleSubcommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::cli::RuleSubcommand;
    match cmd {
        RuleSubcommand::Check { path, json } => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            match crate::semantic::parse_semantic_auto(&text) {
                Ok(doc) => match crate::semantic::validate(&doc) {
                    Ok(()) => {
                        if *json {
                            println!("{}", serde_json::json!({ "status": "ok", "path": path }));
                        } else {
                            println!("OK: {path} is a valid semantic ruleset");
                        }
                        Ok(())
                    }
                    Err(e) => {
                        // Semantic validation failed — fall back to raw `Rules::from_toml`
                        // so `examples/rules/sample.toml` (raw engine format) can still
                        // validate. Only accept the raw fallback if it parses and has
                        // non-empty `when` guards; otherwise surface the semantic error
                        // (e.g. typo'd `in_zne` that would otherwise look like an empty
                        // raw rule and incorrectly pass).
                        if let Ok(rules) = crate::rules::Rules::from_toml(&text)
                            && !rules.rules.is_empty()
                            && rules
                                .rules
                                .iter()
                                .all(|r| !r.when.all.is_empty() || !r.when.any.is_empty())
                        {
                            if *json {
                                println!(
                                    "{}",
                                    serde_json::json!({ "status": "ok", "path": path, "kind": "raw" })
                                );
                            } else {
                                println!("OK: {path} is a valid raw ruleset");
                            }
                            return Ok(());
                        }
                        if *json {
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
                    // Parse failed as semantic — try raw `Rules::from_toml` fallback
                    // before giving up. This lets `flo rule check` accept both
                    // semantic documents and raw engine TOML (e.g. sample.toml).
                    if let Ok(rules) = crate::rules::Rules::from_toml(&text)
                        && !rules.rules.is_empty()
                        && rules
                            .rules
                            .iter()
                            .all(|r| !r.when.all.is_empty() || !r.when.any.is_empty())
                    {
                        // Extra guard: reject raw rules whose topics don't match the
                        // naming convention — typo'd semantic files would otherwise
                        // parse as empty-when raw rules and slip through.
                        let bad_topic = rules
                            .rules
                            .iter()
                            .flat_map(|r| r.when.all.iter().chain(r.when.any.iter()))
                            .find(|t| crate::topic::check_topic_pattern(&t.topic).is_err());
                        if bad_topic.is_none() {
                            if *json {
                                println!(
                                    "{}",
                                    serde_json::json!({ "status": "ok", "path": path, "kind": "raw" })
                                );
                            } else {
                                println!("OK: {path} is a valid raw ruleset");
                            }
                            return Ok(());
                        }
                    }
                    if *json {
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
        RuleSubcommand::Compile {
            path,
            robot_id,
            json: _,
        } => {
            let rid = robot_id.as_deref().unwrap_or("7");
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let doc = crate::semantic::parse_semantic_auto(&text).map_err(|e| {
                eprintln!("PARSE ERROR: {e}");
                std::process::exit(1);
            })?;
            let rules = crate::semantic::compile(&doc, rid).map_err(|e| {
                eprintln!("COMPILE ERROR: {e}");
                std::process::exit(1);
            })?;
            println!("{}", serde_json::to_string_pretty(&rules).unwrap());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn readiness_waits_for_engine_subscription() {
        let health = Health::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut gate = std::pin::pin!(await_engine_ready(rx, &health));
        // The gate must not complete while the engine has not confirmed.
        tokio::select! {
            _ = &mut gate => panic!("readiness gate completed before engine confirmation"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        assert!(!health.is_ready());
        tx.send(()).unwrap();
        assert!(gate.await);
        assert!(health.is_ready());
    }

    #[tokio::test]
    async fn readiness_stays_unset_when_engine_dies_before_subscribing() {
        let health = Health::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Engine dies before confirming: its sender drops without a send.
        drop(tx);
        assert!(!await_engine_ready(rx, &health).await);
        assert!(!health.is_ready());
    }
}
