//! Client runtime: a single deep entry point that owns transport open, auth
//! config application, registration, ruleset loading with fail-safe fallback,
//! and task supervision. Replaces the three competing startup flows (the dead
//! `demo`/`production` modules and the inline startup in `flo-client.rs`).

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{error, info};

use crate::auth::{AuthConfig, AuthMode};
use crate::cli::Args;
use crate::config::{ClientConfig, RuleStore, run_hot_reload};
use crate::engine;
use crate::health;
use crate::health::Health;
#[cfg(feature = "media")]
use crate::mesh::run_signaling;
use crate::mutation::compute_sha;
use crate::registration::{RegistrationError, register_with_client};
use crate::semantic;
use crate::transport::Transport;

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

/// The client runtime. `run` owns the whole client lifecycle: it validates
/// auth (fail-closed), loads the ruleset (fail-safe on missing/invalid input),
/// opens the Zenoh transport, registers with the server, starts the shared
/// subsystems, and supervises them until one dies.
pub struct ClientRuntime;

impl ClientRuntime {
    /// Run the client until a supervised subsystem exits.
    pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let robot_id = args
            .robot_id
            .clone()
            .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
            .unwrap_or_else(|| "7".to_string());

        // Auth is a security gate: fail closed on an invalid setup rather than
        // silently dropping the requested mTLS/ed25519 protection.
        let auth = build_auth(&args)?;

        // Fail-safe input handling: a missing/unreadable/invalid client config
        // or ruleset drops us to safe-state (empty ruleset, no motion commands)
        // instead of exiting — matching the README safety posture.
        let inputs = load_inputs(&args, &robot_id);

        // Transport: auth-derived config (mTLS / none) plus explicit peers.
        let mut config = auth
            .zenoh_config(&robot_id)
            .map_err(|e| format!("auth config invalid: {e}"))?;
        if !args.connect.is_empty() {
            let _ = config.insert_json5("mode", "\"client\"");
            let endpoints: Vec<String> = args.connect.iter().map(|e| format!("\"{e}\"")).collect();
            let _ = config.insert_json5("connect/endpoints", &format!("[{}]", endpoints.join(",")));
        }
        let mut transport = Transport::open_with(config).await?;
        transport.declare_liveliness(&robot_id).await?;
        let transport = Arc::new(transport);
        info!(%robot_id, "zenoh session open, liveliness declared");

        // Register with the server when a valid client config is present; in
        // safe-state there is no config payload to register with.
        if let Some(cfg) = &inputs.client_config {
            info!(%robot_id, "registering with server...");
            match register_with_client(transport.clone(), &robot_id, cfg).await {
                Ok(()) => info!("registration confirmed"),
                Err(RegistrationError::AlreadyRegistered) => {
                    return Err("client already registered with server".into());
                }
                Err(RegistrationError::Poisoned) => {
                    return Err("client is poisoned on server — cannot join".into());
                }
                Err(RegistrationError::NotRegistered) => {
                    return Err("client not registered with server".into());
                }
                Err(RegistrationError::Timeout) => {
                    return Err("registration timed out after 3 retries".into());
                }
                Err(RegistrationError::ServerError(e)) => {
                    return Err(format!("registration rejected: {e}").into());
                }
            }
        }

        // Start the shared subsystems and supervise them: exit when the first
        // subsystem dies so a process supervisor can restart the client.
        let handles = start_common_subsystems(&transport, &inputs.store, &robot_id, &args).await;
        crate::media::spawn_video_peer(&args, transport, robot_id);

        Self::supervise(handles).await
    }

    /// Supervise the client's subsystems until one dies. Mirrors the server's
    /// `tokio::try_join!` discipline: the first dead subsystem is logged as
    /// fatal and the process exits non-zero so a supervisor can restart it.
    pub async fn supervise(
        handles: SubsystemHandles,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(feature = "media")]
        {
            tokio::select! {
                res = handles.health => fatal_exit("health", res),
                res = handles.reload => fatal_exit("hot-reload", res),
                res = handles.engine => fatal_exit("rule engine", res),
                res = handles.signaling => fatal_exit("signaling", res),
            }
        }
        #[cfg(not(feature = "media"))]
        {
            tokio::select! {
                res = handles.health => fatal_exit("health", res),
                res = handles.reload => fatal_exit("hot-reload", res),
                res = handles.engine => fatal_exit("rule engine", res),
            }
        }
    }
}

/// Log a dead subsystem at fatal severity and return the process error. Any
/// completion is fatal: a subsystem that stops running — clean or with an error —
/// must take the whole client down, never leave it alive but unsupervised.
fn fatal_exit(
    subsystem: &str,
    res: Result<(), tokio::task::JoinError>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    error!(
        subsystem = subsystem,
        "fatal: {subsystem} subsystem exited: {res:?}"
    );
    Err(format!("{subsystem} subsystem exited: {res:?}").into())
}

/// Build and validate the auth config from CLI flags. Production validation
/// rejects `auth: none` without `--auth-allow-insecure` and requires credential
/// files for authenticated modes, so an invalid setup is a loud error, never a
/// silent downgrade.
fn build_auth(args: &Args) -> Result<AuthConfig, Box<dyn std::error::Error + Send + Sync>> {
    let auth_mode = AuthMode::parse(&args.auth_mode)
        .map_err(|e| format!("invalid --auth-mode '{0}': {e}", args.auth_mode))?;
    let auth = AuthConfig {
        mode: auth_mode,
        allow_insecure: args.auth_allow_insecure,
        cert: args.auth_cert.clone().map(PathBuf::from),
        key: args.auth_key.clone().map(PathBuf::from),
        trust: args.auth_trust.clone().map(PathBuf::from),
    };
    auth.validate_production()
        .map_err(|e| format!("auth config invalid: {e}"))?;
    if auth.mode.is_authenticated() {
        info!(mode = ?auth.mode, "auth validated (authenticated client)");
    } else {
        tracing::warn!(
            "auth: none permitted via --auth-allow-insecure; NO impersonation protection"
        );
    }
    Ok(auth)
}

/// Client config plus the ruleset store. In safe-state the config is `None`
/// (registration is skipped) and the store holds the empty fail-safe ruleset.
struct Inputs {
    client_config: Option<ClientConfig>,
    store: RuleStore,
}

/// Load the client config file and ruleset with fail-safe fallback. A
/// missing/unreadable/invalid config or ruleset logs `safe-state` and falls
/// back to an empty ruleset (no motion commands) instead of hard-exiting.
fn load_inputs(args: &Args, robot_id: &str) -> Inputs {
    let client_config = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match ClientConfig::from_toml(&text) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    error!(
                        path = %path,
                        error = %e,
                        "safe-state: client config invalid — empty ruleset, no motion commands"
                    );
                    None
                }
            },
            Err(e) => {
                error!(
                    path = %path,
                    error = %e,
                    "safe-state: client config unreadable — empty ruleset, no motion commands"
                );
                None
            }
        },
        None => {
            error!("safe-state: missing client config — empty ruleset, no motion commands");
            None
        }
    };

    // Built-in demo rules only when no ruleset was requested and the client
    // config is valid; any requested-but-broken ruleset lands in safe-state.
    let store = match &args.ruleset {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => {
                let sha = compute_sha(text.as_bytes());
                info!(%robot_id, %sha, "ruleset loaded");
                compile_rules_or_default(&text, robot_id)
            }
            Err(e) => {
                error!(
                    path = %path,
                    error = %e,
                    "safe-state: ruleset unreadable — no motion commands"
                );
                fail_safe_store()
            }
        },
        None if client_config.is_some() => {
            info!("no ruleset file — using built-in demo");
            RuleStore::bootstrap_demo(robot_id)
        }
        None => fail_safe_store(),
    };

    Inputs {
        client_config,
        store,
    }
}

/// The minimal fail-safe ruleset: no motion commands are emitted.
fn fail_safe_store() -> RuleStore {
    RuleStore::bootstrap("rules = []\n").expect("empty ruleset always parses")
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_rules_or_default(text: &str, robot_id: &str) -> RuleStore {
    if let Ok(doc) = semantic::parse_semantic(text) {
        match semantic::compile(&doc, robot_id) {
            Ok(rules) => match RuleStore::bootstrap(&rules.to_toml()) {
                Ok(s) => return s,
                Err(e) => {
                    error!(error = %e, "semantic compile produced invalid rules -> safe-state")
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "semantic compile failed; falling back to raw TOML")
            }
        }
    }
    match RuleStore::bootstrap(text) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "config invalid -> starting in fail-safe safe-state");
            fail_safe_store()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::time::Duration;

    fn args_from(argv: &[&str]) -> Args {
        Args::parse_from(argv)
    }

    #[test]
    fn auth_none_requires_insecure_override() {
        let blocked = args_from(&["flo", "--auth-mode", "none"]);
        assert!(build_auth(&blocked).is_err());

        let allowed = args_from(&["flo", "--auth-mode", "none", "--auth-allow-insecure"]);
        assert!(build_auth(&allowed).is_ok());
    }

    #[test]
    fn auth_mtls_requires_credentials() {
        // Default auth mode is mtls; without cert/key/trust this must fail
        // closed rather than silently running unauthenticated.
        let args = args_from(&["flo"]);
        assert!(build_auth(&args).is_err());
    }

    #[test]
    fn auth_unknown_mode_rejected() {
        let args = args_from(&["flo", "--auth-mode", "kerberos"]);
        assert!(build_auth(&args).is_err());
    }

    #[tokio::test]
    async fn missing_config_uses_empty_ruleset() {
        let args = args_from(&["flo"]);
        let store = load_inputs(&args, "robot-7").store;
        assert_eq!(store.current().await.rules.len(), 0);
    }

    #[tokio::test]
    async fn valid_config_without_ruleset_uses_demo_rules() {
        let dir = std::env::temp_dir();
        let path = dir.join("flo-runtime-config.toml");
        std::fs::write(
            &path,
            r#"
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
"#,
        )
        .unwrap();
        let args = args_from(&["flo", "--config", &path.to_string_lossy()]);
        let inputs = load_inputs(&args, "robot-7");
        assert!(inputs.client_config.is_some());
        assert_eq!(inputs.store.current().await.rules.len(), 2);
    }

    #[tokio::test]
    async fn missing_ruleset_file_falls_back_to_safe_state() {
        let args = args_from(&["flo", "--ruleset", "/nonexistent/rules.toml"]);
        let store = load_inputs(&args, "robot-7").store;
        assert_eq!(store.current().await.rules.len(), 0);
    }

    #[tokio::test]
    async fn unreadable_config_falls_back_to_safe_state() {
        let args = args_from(&["flo", "--config", "/nonexistent/config.toml"]);
        let inputs = load_inputs(&args, "robot-7");
        assert!(inputs.client_config.is_none());
        assert_eq!(inputs.store.current().await.rules.len(), 0);
    }

    const VALID_TOML: &str = r#"
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot/7/local/bumper", pred = { Comparison = { op = "Eq", lhs = { Str = "pressed" }, rhs = { Bool = true } } } },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
"#;

    const SEMANTIC_DOC: &str = r#"
[site]
id = "cell-7"
frame = "cell-7/world"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
"#;

    #[tokio::test]
    async fn compiles_valid_raw_rules() {
        let store = compile_rules_or_default(VALID_TOML, "robot-7");
        assert_eq!(store.current().await.rules.len(), 1);
    }

    #[tokio::test]
    async fn compiles_semantic_doc() {
        // Semantic doc goes through semantic::compile, not the raw TOML fallback.
        let store = compile_rules_or_default(SEMANTIC_DOC, "robot-7");
        assert_eq!(store.current().await.rules.len(), 1);
    }

    #[tokio::test]
    async fn garbage_falls_back_to_fail_safe_state() {
        // Neither a semantic doc nor raw TOML: must land in safe-state (0 rules,
        // no motion commands).
        let store = compile_rules_or_default("this is {{{ not toml at all", "robot-7");
        assert_eq!(store.current().await.rules.len(), 0);
    }

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
