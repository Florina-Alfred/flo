//! Production mode: file-based rules, optional external config, no simulation
//! unless explicitly requested (e.g. a dev node without hardware).

use std::sync::Arc;

use tracing::info;

use crate::auth::{AuthConfig, AuthMode};
use crate::config::RuleStore;
use crate::semantic;
use crate::transport::Transport;

use crate::cli::Args;
use crate::common::{block_indefinitely, spawn_video_peer, start_common_subsystems};

/// Run in production mode (k8s DaemonSet): load rules from `--config`, open a
/// real zenoh session, and start the shared subsystems.
pub async fn run_production(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(robot_id, "starting flo client (production mode)");

    // Build + validate the auth config before opening any session. Production
    // hard-blocks `auth: none` unless explicitly overridden; this fails fast.
    let auth_mode = AuthMode::parse(&args.auth_mode)
        .map_err(|e| format!("invalid --auth-mode '{0}': {e}", args.auth_mode))?;
    let auth = AuthConfig {
        mode: auth_mode,
        allow_insecure: args.auth_allow_insecure,
        cert: args.auth_cert.clone().map(std::path::PathBuf::from),
        key: args.auth_key.clone().map(std::path::PathBuf::from),
        trust: args.auth_trust.clone().map(std::path::PathBuf::from),
    };
    if auth.mode.is_authenticated() {
        auth.validate_production()
            .map_err(|e| format!("auth config invalid: {e}"))?;
        info!(mode = ?auth.mode, "auth validated (authenticated client)");
    } else {
        match auth.validate_production() {
            Ok(_) => tracing::warn!(
                "auth: none permitted via --auth-allow-insecure; NO impersonation protection"
            ),
            Err(_) => {
                return Err(
                    "auth: none is blocked in production; set --auth-allow-insecure for dev/air-gapped only"
                        .into(),
                )
            }
        }
    }

    let bootstrap = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(path, error = %e, "config unreadable -> starting in fail-safe safe-state (no unrestricted motion)");
                empty_ruleset_toml()
            }
        },
        None => "rules = []\n".to_string(),
    };

    // Try semantic (extended-TOML) first; fall back to raw TOML; else safe-state.
    let store = compile_rules_or_default(&bootstrap, &robot_id);

    let mut transport = Transport::open_with(
        auth.zenoh_config(&robot_id)
            .map_err(|e| format!("auth config invalid: {e}"))?,
    )
    .await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "zenoh session open, liveliness declared");

    start_common_subsystems(&transport, &store, &robot_id, &args).await;

    spawn_video_peer(&args, transport, robot_id);

    block_indefinitely().await;
    Ok(())
}

/// A minimal fail-safe ruleset: no motion commands are emitted.
fn empty_ruleset_toml() -> String {
    "rules = []\n".to_string()
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_rules_or_default(text: &str, robot_id: &str) -> RuleStore {
    if let Ok(doc) = semantic::parse_semantic(text) {
        match semantic::compile(&doc, robot_id) {
            Ok(rules) => match RuleStore::bootstrap(&rules.to_toml()) {
                Ok(s) => return s,
                Err(e) => {
                    tracing::error!(error = %e, "semantic compile produced invalid rules -> safe-state")
                }
            },
            Err(e) => tracing::error!(error = %e, "semantic validation failed -> safe-state"),
        }
    }
    match RuleStore::bootstrap(text) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "config invalid -> starting in fail-safe safe-state");
            RuleStore::bootstrap(&empty_ruleset_toml()).expect("empty ruleset always parses")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // A minimal semantic (extended-TOML) doc, mirroring tests/semantic_compile.rs.
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
}
