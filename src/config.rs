use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::registry::{RegisterOutcome, Registry};
use crate::rules::{Rules, Ruleset};
use crate::transport::Transport;

/// Shared, atomically-swappable ruleset. Readers hold an `Arc` clone; a hot-reload
/// replaces the inner `Arc` without disturbing in-flight evaluations.
#[derive(Clone)]
pub struct RuleStore {
    inner: Arc<RwLock<Arc<Rules>>>,
}

impl RuleStore {
    /// Create a store from an already-compiled ruleset.
    pub fn new(rules: Arc<Rules>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(rules)),
        }
    }

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
        const DEMO: &str = r#"
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot/{id}/local/bumper", pred = { Comparison = { op = "Eq", lhs = { Str = "pressed" }, rhs = { Bool = true } } } },
  { topic = "robot/{id}/local/imu",    pred = { Comparison = { op = "Gt", lhs = { Str = "speed_mps" }, rhs = { Float = 0.2 } } } },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]

[[rules]]
name = "lidar-block-slowdown"
when.any = [
  { topic = "lidar/fleet/scan", pred = { Comparison = { op = "Lt", lhs = { Str = "min_range_m" }, rhs = { Float = 0.5 } } } },
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

/// Server-mode hot-reload: subscribe to the fleet-scoped ruleset publish topic,
/// validate each incoming [`Ruleset`] against the [`Registry`], and only swap
/// the store on `Inserted`/`Updated`. Rejects with conflict are logged (last-good
/// preserved); parse errors are logged (last-good preserved).
pub async fn run_hot_reload_with_registry(
    transport: &Transport,
    robot_id: &str,
    store: RuleStore,
    registry: Arc<Registry>,
) -> zenoh::Result<()> {
    use crate::transport::RULESET_PUB_KEY;

    let wildcard_key = RULESET_PUB_KEY
        .replace("{site}", "*")
        .replace("{name}", "**");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<zenoh::sample::Sample>();
    transport
        .subscribe(&wildcard_key, move |sample: zenoh::sample::Sample| {
            let _ = tx.send(sample);
        })
        .await?;
    info!(topic = %wildcard_key, "hot-reload subscriber active (registry)"); // cspell:disable-line

    while let Some(sample) = rx.recv().await {
        let bytes = sample.payload().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        match Ruleset::from_toml(&text) {
            Ok(rs) => match registry.publish(&rs, robot_id) {
                Ok(RegisterOutcome::Inserted) | Ok(RegisterOutcome::Updated { .. }) => {
                    match Rules::from_toml(&rs.to_toml()) {
                        Ok(rules) => {
                            let n = rules.rules.len();
                            store.swap(Arc::new(rules)).await;
                            info!(
                                rules = n,
                                name = %rs.ruleset_name,
                                "ruleset hot-reloaded via registry"
                            );
                        }
                        Err(e) => error!(error = %e, "compiled ruleset invalid; keeping previous"),
                    }
                }
                Ok(RegisterOutcome::RejectedConflict) => {
                    error!("ruleset rejected: owner conflict; keeping previous");
                }
                Ok(RegisterOutcome::Quarantined) => {
                    error!("ruleset quarantined; keeping previous");
                }
                Err(e) => error!(error = %e, "registry error; keeping previous"),
            },
            Err(e) => error!(error = %e, "rejected bad ruleset update; keeping previous"),
        }
    }
    Ok(())
}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub client: ClientSection,
    #[serde(default)]
    pub server: Option<ServerSection>,
    #[serde(default)]
    pub default_subscriptions: Option<DefaultSubscriptions>,
    #[serde(default)]
    pub default_publishers: Option<DefaultPublishers>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSection {
    pub heartbeat_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerSection {
    #[serde(default)]
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultSubscriptions {
    pub location: Option<LocationSubscriptions>,
    pub zone: Option<ZoneSubscriptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationSubscriptions {
    pub x: String,
    pub y: String,
    pub z: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneSubscriptions {
    pub site_id: String,
    pub zone_enter: String,
    pub zone_exit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultPublishers {
    pub location: Option<PublisherConfig>,
    pub zone: Option<PublisherConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherConfig {
    pub topic: String,
    pub period_ms: u64,
}

impl ClientConfig {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let cfg: ClientConfig =
            toml::from_str(text).map_err(|e| format!("invalid client config TOML: {e}"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), String> {
        let subs = self
            .default_subscriptions
            .as_ref()
            .ok_or("missing [default_subscriptions] table")?;

        let loc = subs
            .location
            .as_ref()
            .ok_or("missing [default_subscriptions.location] table")?;
        if loc.x.is_empty() {
            return Err("default_subscriptions.location.x is empty".into());
        }
        if loc.y.is_empty() {
            return Err("default_subscriptions.location.y is empty".into());
        }
        if loc.z.is_empty() {
            return Err("default_subscriptions.location.z is empty".into());
        }

        let zone = subs
            .zone
            .as_ref()
            .ok_or("missing [default_subscriptions.zone] table")?;
        if zone.site_id.is_empty() {
            return Err("default_subscriptions.zone.site_id is empty".into());
        }
        if zone.zone_enter.is_empty() {
            return Err("default_subscriptions.zone.zone_enter is empty".into());
        }
        if zone.zone_exit.is_empty() {
            return Err("default_subscriptions.zone.zone_exit is empty".into());
        }

        let pubs = self
            .default_publishers
            .as_ref()
            .ok_or("missing [default_publishers] table")?;

        let pub_loc = pubs
            .location
            .as_ref()
            .ok_or("missing [default_publishers.location] table")?;
        if pub_loc.topic.is_empty() {
            return Err("default_publishers.location.topic is empty".into());
        }

        let pub_zone = pubs
            .zone
            .as_ref()
            .ok_or("missing [default_publishers.zone] table")?;
        if pub_zone.topic.is_empty() {
            return Err("default_publishers.zone.topic is empty".into());
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub expected_clients: Vec<ExpectedClient>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedClient {
    pub robot_id: String,
}

impl ServerConfig {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(ServerConfig::default());
        }
        toml::from_str(text).map_err(|e| format!("invalid server config TOML: {e}"))
    }
}
