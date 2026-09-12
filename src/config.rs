use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info};
use zenoh::key_expr::KeyExpr;

use crate::registry::{RegisterOutcome, Registry};
use crate::rules::{Rules, Ruleset};
use crate::transport::Transport;

/// Shared, atomically-swappable ruleset. Readers hold an `Arc` clone; a hot-reload
/// replaces the inner `Arc` without disturbing in-flight evaluations.
#[derive(Clone)]
pub struct ActiveRules {
    inner: Arc<RwLock<Arc<Rules>>>,
}

/// Alias for one release — use [`ActiveRules`] for new code.
pub type RuleStore = ActiveRules;

impl ActiveRules {
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
        const DEMO: &str = include_str!("../examples/rules/hrc-demo.toml");
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
    pub async fn swap(&self, rules: Arc<Rules>) {
        *self.inner.write().await = rules;
    }
}

/// Unified hot-reload subscriber. `policy` decides the mode:
/// - `None` => per-robot `Rules::from_toml` on `robot/{id}/local/rules`.
/// - `Some(registry)` => fleet `Ruleset::from_toml` → `registry.publish` → `Rules::from_toml` on `fleet/*/ruleset/**`.
pub struct HotReload {
    transport: Arc<Transport>,
    store: ActiveRules,
    policy: Option<Arc<Registry>>,
    topic: KeyExpr<'static>,
    robot_id: String,
}

impl HotReload {
    pub fn new(
        transport: Arc<Transport>,
        store: ActiveRules,
        policy: Option<Arc<Registry>>,
        topic: impl Into<KeyExpr<'static>>,
        robot_id: impl Into<String>,
    ) -> Self {
        Self {
            transport,
            store,
            policy,
            topic: topic.into(),
            robot_id: robot_id.into(),
        }
    }

    /// Run the hot-reload loop. A malformed update is rejected (old rules stay active) and logged.
    pub async fn run(&self) -> zenoh::Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<zenoh::sample::Sample>();
        let key = self.topic.clone();
        self.transport
            .subscribe(key.as_str(), move |sample: zenoh::sample::Sample| {
                let _ = tx.send(sample);
            })
            .await?;
        if self.policy.is_some() {
            info!(topic = %self.topic, "hot-reload subscriber active (registry)");
        } else {
            info!(topic = %self.topic, "hot-reload subscriber active");
        }

        while let Some(sample) = rx.recv().await {
            let bytes = sample.payload().to_bytes();
            let text = String::from_utf8_lossy(&bytes);
            if let Some(registry) = &self.policy {
                match Ruleset::from_toml(&text) {
                    Ok(rs) => match registry.publish(&rs, &self.robot_id) {
                        Ok(RegisterOutcome::Inserted) | Ok(RegisterOutcome::Updated { .. }) => {
                            match Rules::from_toml(&rs.to_toml()) {
                                Ok(rules) => {
                                    let n = rules.rules.len();
                                    self.store.swap(Arc::new(rules)).await;
                                    info!(
                                        rules = n,
                                        name = %rs.ruleset_name,
                                        "ruleset hot-reloaded via registry"
                                    );
                                }
                                Err(e) => {
                                    error!(error = %e, "compiled ruleset invalid; keeping previous")
                                }
                            }
                        }
                        Ok(RegisterOutcome::RejectedConflict) => {
                            error!("ruleset rejected: owner conflict; keeping previous");
                        }
                        Err(e) => error!(error = %e, "registry error; keeping previous"),
                    },
                    Err(e) => error!(error = %e, "rejected bad ruleset update; keeping previous"),
                }
            } else {
                match Rules::from_toml(&text) {
                    Ok(rules) => {
                        let n = rules.rules.len();
                        self.store.swap(Arc::new(rules)).await;
                        info!(rules = n, "ruleset hot-reloaded");
                    }
                    Err(e) => error!(error = %e, "rejected bad ruleset update; keeping previous"),
                }
            }
        }
        Ok(())
    }
}

/// Subscribe to the zenoh hot-reload topic and swap the store on each update.
/// A malformed TOML update is rejected (old rules stay active) and logged.
pub async fn run_hot_reload(
    transport: &Transport,
    robot_id: &str,
    store: ActiveRules,
) -> zenoh::Result<()> {
    let key = crate::topic::rules_key(robot_id);
    let owned = Arc::new(Transport::from_arc_session(transport.session_arc()));
    HotReload::new(
        owned,
        store,
        None,
        KeyExpr::new(key).unwrap(),
        robot_id.to_string(),
    )
    .run()
    .await
}

/// Server-mode hot-reload: subscribe to the fleet-scoped ruleset publish topic,
/// validate each incoming [`Ruleset`] against the [`Registry`], and only swap
/// the store on `Inserted`/`Updated`. Rejects with conflict are logged (last-good
/// preserved); parse errors are logged (last-good preserved).
pub async fn run_hot_reload_with_registry(
    transport: &Transport,
    robot_id: &str,
    store: ActiveRules,
    registry: Arc<Registry>,
) -> zenoh::Result<()> {
    let owned = Arc::new(Transport::from_arc_session(transport.session_arc()));
    HotReload::new(
        owned,
        store,
        Some(registry),
        KeyExpr::new(crate::topic::RULESET_PUB_PATTERN).unwrap(),
        robot_id.to_string(),
    )
    .run()
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ClientSection {
    pub heartbeat_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ServerSection {
    #[serde(default)]
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultSubscriptions {
    pub location: Option<LocationSubscriptions>,
    pub zone: Option<ZoneSubscriptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSubscriptions {
    pub x: String,
    pub y: String,
    pub z: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSubscriptions {
    pub site_id: String,
    pub zone_enter: String,
    pub zone_exit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultPublishers {
    pub location: Option<PublisherConfig>,
    pub zone: Option<PublisherConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default)]
    pub expected_clients: Vec<ExpectedClient>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CLIENT: &str = r#"
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
"#;

    #[test]
    fn client_parses_valid_toml() {
        let cfg = ClientConfig::from_toml(VALID_CLIENT).expect("valid client TOML");
        assert_eq!(cfg.client.heartbeat_interval_ms, 1000);
        let subs = cfg.default_subscriptions.expect("subscriptions present");
        assert_eq!(
            subs.location.expect("location present").x,
            "robot-7/location/x"
        );
        assert_eq!(subs.zone.expect("zone present").site_id, "robot-7/site");
        let pubs = cfg.default_publishers.expect("publishers present");
        assert_eq!(pubs.location.expect("pub loc").topic, "robot-7/location");
        assert_eq!(pubs.zone.expect("pub zone").period_ms, 1000);
    }

    #[test]
    fn client_rejects_missing_required_section() {
        let err = ClientConfig::from_toml("[client]\nheartbeat_interval_ms = 1000\n")
            .expect_err("missing sections must fail");
        assert!(
            err.contains("missing [default_subscriptions]"),
            "got: {err}"
        );
    }

    #[test]
    fn client_rejects_empty_required_topic() {
        let bad = VALID_CLIENT.replace(r#"topic = "robot-7/location""#, "topic = \"\"");
        let err = ClientConfig::from_toml(&bad).expect_err("empty topic must fail");
        assert!(err.contains("location.topic is empty"), "got: {err}");
    }

    #[test]
    fn client_rejects_unknown_field() {
        let bad = format!("{VALID_CLIENT}\nbogus = true\n");
        assert!(ClientConfig::from_toml(&bad).is_err());
    }

    #[test]
    fn client_rejects_wrong_field_type() {
        let bad = "[client]\nheartbeat_interval_ms = \"not-a-number\"\n";
        assert!(ClientConfig::from_toml(bad).is_err());
    }

    #[test]
    fn server_empty_returns_default() {
        let cfg = ServerConfig::from_toml("").unwrap();
        assert!(cfg.expected_clients.is_empty());
    }

    #[test]
    fn server_parses_expected_clients() {
        let cfg =
            ServerConfig::from_toml("[[expected_clients]]\nrobot_id = \"robot-7\"\n").unwrap();
        assert_eq!(cfg.expected_clients.len(), 1);
        assert_eq!(cfg.expected_clients[0].robot_id, "robot-7");
    }

    #[test]
    fn server_rejects_malformed_toml() {
        assert!(ServerConfig::from_toml("[[expected_clients\n").is_err());
    }

    #[test]
    fn server_rejects_unknown_field() {
        assert!(ServerConfig::from_toml("bogus = 1\n").is_err());
    }

    #[test]
    fn server_rejects_expected_client_without_robot_id() {
        // `robot_id` is a required field: an expected-client entry that omits it
        // must fail closed rather than silently allow an anonymous client.
        assert!(ServerConfig::from_toml("[[expected_clients]]\n").is_err());
    }

    #[tokio::test]
    async fn bootstrap_parses_empty_ruleset() {
        let store = ActiveRules::bootstrap("rules = []\n").expect("empty rules parse");
        assert_eq!(store.current().await.rules.len(), 0);
        let demo = ActiveRules::bootstrap_demo("robot-7");
        assert_eq!(demo.current().await.rules.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hot_reload_swaps_via_transport() {
        // Verify ActiveRules::swap/current via the HotReload path (not just unit RwLock).
        let transport = Arc::new(
            crate::transport::Transport::open_with(crate::transport::Transport::loopback_config())
                .await
                .expect("open loopback"),
        );
        let store = ActiveRules::bootstrap("rules = []\n").unwrap();
        let hr = HotReload::new(
            transport.clone(),
            store.clone(),
            None,
            KeyExpr::new(crate::topic::rules_key("7")).unwrap(),
            "7".to_string(),
        );
        let h = tokio::spawn(async move {
            let _ = hr.run().await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let new_toml = r#"
[[rules]]
name = "hr-test"
when.all = [{ topic = "robot/7/local/bumper", mode = "Level" }]
actions = [{ topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } }]
"#;
        transport
            .put_bytes(&crate::topic::rules_key("7"), new_toml.as_bytes().to_vec())
            .await
            .expect("put toml");
        let mut ok = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if store.current().await.rules.len() == 1 {
                ok = true;
                break;
            }
        }
        assert!(ok, "HotReload should have swapped store to 1 rule");
        h.abort();
    }
}
