//! Client runtime: a single deep entry point that owns transport open, auth
//! config application, registration, ruleset loading with fail-safe fallback,
//! and task supervision.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::auth::{AuthConfig, AuthMode};
use crate::cli::Args;
use crate::config::{ActiveRules, ClientConfig, run_hot_reload};
use crate::engine;
use crate::health::{Health, ReadyGate, Supervisor};
#[cfg(feature = "media")]
use crate::mesh::run_signaling;
use crate::mutation::compute_sha;
use crate::registration::{RegistrationError, register_with_client};
use crate::semantic;
use crate::transport::Transport;

/// Handles to the spawned subsystems, for supervision by the runtime.
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
/// Readiness is gated on the rule engine consuming the `ReadyGate` token:
/// `/readyz` flips 200 only after the engine's initial sensor topics are live,
/// so the probe never reports ready while the engine is still subscribing
/// (or dead).
///
/// `ready_gate` is the shared readiness token. The engine clones it and flips
/// it after subscribing; the health server holds a clone to report readiness.
pub async fn start_common_subsystems(
    transport: &Arc<Transport>,
    store: &ActiveRules,
    robot_id: &str,
    #[cfg_attr(not(feature = "media"), allow(unused_variables))] args: &Args,
    ready_gate: ReadyGate,
) -> SubsystemHandles {
    let health_task = {
        let health = ready_gate.clone();
        tokio::spawn(async move {
            let addr = std::env::var("FLO_HEALTH_ADDR").unwrap_or_else(|_| "0.0.0.0:0".to_string());
            if let Err(e) = crate::health::serve(health, &addr).await {
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
        let gate = ready_gate.clone();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(transport, store, gate).await {
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

    SubsystemHandles {
        health: health_task,
        reload: reload_task,
        engine: engine_task,
        #[cfg(feature = "media")]
        signaling: signal_task,
    }
}

/// Deep runtime that owns `Transport + ActiveRules + Health` atomically.
/// Replaces the empty `ClientRuntime` god-function with a `bootstrap` that
/// collapses the six bounces (`Args` → `build_auth` → `load_inputs` →
/// `zenoh_config` → `Transport::open` → `declare_liveliness` → `register`)
/// into one deep entry point, exposing a `ReadyGate` token for the engine.
pub struct Runtime {
    pub transport: Arc<Transport>,
    pub store: ActiveRules,
    pub health: Health,
}

impl Runtime {
    /// Bootstrap the runtime from `Args`, owning transport open, liveliness and
    /// registration atomically. Returns the deep `Runtime` and the readiness
    /// gate token that the engine will consume.
    pub async fn bootstrap(
        args: Args,
    ) -> Result<(Self, ReadyGate), Box<dyn std::error::Error + Send + Sync>> {
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
        // Deep adapter: callers never touch the low-level config — endpoint merging is inside `Transport`.
        let config = auth
            .zenoh_config(&robot_id)
            .map_err(|e| format!("auth config invalid: {e}"))?;
        let config = if args.connect.is_empty() {
            config
        } else {
            Transport::with_endpoints(config, &args.connect)
        };
        let mut transport = Transport::open_with(config).await?;
        let locators = transport.locators().await;
        info!(locators = ?locators, %robot_id, "zenoh session open");
        transport.declare_liveliness(&robot_id).await?;
        let transport = Arc::new(transport);
        info!(%robot_id, "liveliness declared");

        // Register with the server when a valid client config is present; in
        // safe-state there is no config payload to register with.
        // Retry/backoff is caller policy (ARCH-01): the transport's
        // `register_with_client` is blocking single-attempt; this loop owns
        // the `3× + 1s*attempt` backoff.
        if let Some(cfg) = &inputs.client_config {
            info!(%robot_id, "registering with server...");
            const REGISTRATION_RETRIES: u32 = 3;
            const RETRY_BACKOFF_MS: u64 = 1000;
            for attempt in 1..=REGISTRATION_RETRIES {
                match register_with_client(transport.clone(), &robot_id, cfg).await {
                    Ok(()) => {
                        info!("registration confirmed");
                        break;
                    }
                    Err(RegistrationError::Timeout) => {
                        if attempt < REGISTRATION_RETRIES {
                            warn!(
                                attempt,
                                %robot_id,
                                "registration not acknowledged, retrying..."
                            );
                            tokio::time::sleep(Duration::from_millis(
                                RETRY_BACKOFF_MS * attempt as u64,
                            ))
                            .await;
                        } else {
                            return Err("registration timed out after 3 retries".into());
                        }
                    }
                    Err(RegistrationError::AlreadyRegistered) => {
                        return Err("client already registered with server".into());
                    }
                    Err(RegistrationError::Poisoned) => {
                        return Err("client is poisoned on server — cannot join".into());
                    }
                    Err(RegistrationError::NotRegistered) => {
                        return Err("client not registered with server".into());
                    }
                    Err(RegistrationError::ServerError(e)) => {
                        return Err(format!("registration rejected: {e}").into());
                    }
                }
            }
        }

        let health = ReadyGate::new();
        let ready_gate = health.clone();
        let runtime = Self {
            transport,
            store: inputs.store,
            health,
        };
        Ok((runtime, ready_gate))
    }

    /// Run the runtime until a supervised subsystem exits. Collapses the old
    /// `ClientRuntime::run` god-function into a deep method that owns the
    /// lifecycle via `Supervisor`.
    pub async fn run(self, args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let robot_id = args
            .robot_id
            .clone()
            .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
            .unwrap_or_else(|| "7".to_string());
        let gate = self.health.clone();
        let handles =
            start_common_subsystems(&self.transport, &self.store, &robot_id, &args, gate).await;
        crate::media::spawn_video_peer(&args, self.transport.clone(), robot_id);
        Self::supervise(handles).await
    }

    /// Supervise the client's subsystems until one dies. Delegates to the
    /// single `Supervisor` shared with the server so health is always
    /// supervised.
    pub async fn supervise(
        handles: SubsystemHandles,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut vec: Vec<(&'static str, tokio::task::JoinHandle<()>)> = Vec::new();
        vec.push(("health", handles.health));
        vec.push(("hot-reload", handles.reload));
        vec.push(("rule engine", handles.engine));
        #[cfg(feature = "media")]
        vec.push(("signaling", handles.signaling));
        Supervisor::await_shutdown(vec).await
    }
}

/// Backwards-compatible alias for `Runtime`. Kept for one release so
/// integration tests importing `ClientRuntime` keep compiling; new code should
/// use `Runtime`.
pub type ClientRuntime = Runtime;

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
    store: ActiveRules,
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
            ActiveRules::bootstrap_demo(robot_id)
        }
        None => fail_safe_store(),
    };

    Inputs {
        client_config,
        store,
    }
}

/// The minimal fail-safe ruleset: no motion commands are emitted.
fn fail_safe_store() -> ActiveRules {
    ActiveRules::bootstrap("rules = []\n").expect("empty ruleset always parses")
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_rules_or_default(text: &str, robot_id: &str) -> ActiveRules {
    if let Ok(doc) = semantic::parse_semantic(text) {
        match semantic::compile(&doc, robot_id) {
            Ok(rules) => match ActiveRules::bootstrap(&rules.to_toml()) {
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
    match ActiveRules::bootstrap(text) {
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
        // ReadyGate is now the single readiness interface; the engine flips it
        // after subscribing. Test that the gate starts not-ready and flips on
        // set_ready, mirroring the old oneshot-gated readiness.
        let gate = ReadyGate::new();
        assert!(!gate.is_ready());
        // Simulate engine confirming subscriptions.
        gate.set_ready();
        assert!(gate.is_ready());
        // Subsequent readiness stays true.
        assert!(gate.is_ready());
        // A fresh gate is still not-ready.
        let fresh = ReadyGate::new();
        assert!(!fresh.is_ready());
    }

    #[tokio::test]
    async fn readiness_stays_unset_when_engine_dies_before_subscribing() {
        // If the engine dies before calling set_ready, readiness stays false.
        let gate = ReadyGate::new();
        // Engine task drops without ever calling set_ready.
        drop(gate.clone());
        // Original gate is still not-ready.
        assert!(!gate.is_ready());
        // Sleep a tick to ensure no async flip.
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!gate.is_ready());
    }
}
