use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{error, info};


use crate::rules::Rules;
use crate::transport::Transport;

/// Shared, atomically-swappable ruleset. Readers hold an `Arc` clone; a hot-reload
/// replaces the inner `Arc` without disturbing in-flight evaluations.
#[derive(Clone)]
pub struct RuleStore {
    inner: Arc<RwLock<Arc<Rules>>>,
}

impl RuleStore {
    /// Bootstrap from TOML text (e.g. the ConfigMap mount). A bad parse is fatal at
    /// startup so misconfiguration fails fast rather than silently running stale rules.
    pub fn bootstrap(toml_text: &str) -> Result<Self, toml::de::Error> {
        let rules = Rules::from_toml(toml_text)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(Arc::new(rules))),
        })
    }

    /// Bootstrap with the built-in demo ruleset (mirrors map-02's example rules), so
    /// `cargo run` with no args shows a rule firing immediately — no config file.
    /// `{id}` placeholders in the rules are rewritten to `robot_id`.
    pub fn bootstrap_demo(robot_id: &str) -> Self {
        // e-stop-on-bumper: bumper pressed AND moving -> reliable STOP.
        // lidar-block-slowdown: lidar min range < 0.5 -> best-effort slowdown.
        const DEMO: &str = r#"
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot/{id}/local/bumper", pred = "pressed == true" },
  { topic = "robot/{id}/local/imu",     pred = "speed_mps > 0.2" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]

[[rules]]
name = "lidar-block-slowdown"
when.any = [
  { topic = "lidar/fleet/scan", pred = "min_range_m < 0.5" },
]
actions = [
  { topic = "robot/{id}/local/drive", qos = "best_effort", payload = { speed_mps = 0.1 } },
]
"#;
        let toml = DEMO.replace("{id}", robot_id);
        let rules = Rules::from_toml(&toml).expect("built-in demo rules must parse");
        Self {
            inner: Arc::new(RwLock::new(Arc::new(rules))),
        }
    }

    /// Read the current active ruleset (cheap `Arc` clone; no copy of rule data).
    pub async fn current(&self) -> Arc<Rules> {
        self.inner.read().await.clone()
    }

    /// Atomically swap in a new ruleset. In-flight holders keep their old `Arc`.
    async fn swap(&self, rules: Arc<Rules>) {
        *self.inner.write().await = rules;
    }
}

/// Subscribe to the zenoh hot-reload topic and swap the store on each update.
/// A malformed TOML update is rejected (old rules stay active) and logged.
pub async fn run_hot_reload(
    transport: &Transport,
    robot_id: &str,
    store: RuleStore,
) -> zenoh::Result<()> {
    let key = crate::transport::RULES_KEY.replace("{id}", robot_id);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<zenoh::sample::Sample>();
    transport
        .subscribe(&key, move |sample: zenoh::sample::Sample| {
            let _ = tx.send(sample);
        })
        .await?;
    info!(topic = %key, "hot-reload subscriber active");

    // Stream updates; swap the store atomically on each valid TOML payload.
    while let Some(sample) = rx.recv().await {
        let bytes = sample.payload().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        match Rules::from_toml(&text) {
            Ok(rules) => {
                let n = rules.rules.len();
                store.swap(Arc::new(rules)).await;
                info!(rules = n, "ruleset hot-reloaded");
            }
            Err(e) => error!(error = %e, "rejected bad ruleset update; keeping previous"),
        }
    }
    Ok(())
}
