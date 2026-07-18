use serde::Deserialize;

/// QoS class a published action targets. Maps onto the locked transport decision:
/// `reliable` => Zenoh class 1 (STOP), `best_effort` => Zenoh class 2 (lidar).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Qos {
    Reliable,
    BestEffort,
}

/// A single publish action fired when a rule's `when` evaluates true.
#[derive(Debug, Clone, Deserialize)]
pub struct Action {
    /// Target key-expression, e.g. `stop/fleet/cmd` or `robot/7/local/drive`.
    pub topic: String,
    /// QoS class for the publish.
    pub qos: Qos,
    /// Free-form payload shipped with the publish (serialized as JSON bytes).
    pub payload: serde_json::Value,
}

/// One predicate: a key-expression match plus an optional predicate string
/// evaluated against the received payload.
#[derive(Debug, Clone, Deserialize)]
pub struct Trigger {
    /// Key-expression the incoming sample must match, e.g. `robot/7/local/bumper`.
    pub topic: String,
    /// Optional predicate over the payload, e.g. `pressed == true`.
    /// Evaluated against a `serde_json::Value` context; unimplemented predicates
    /// (no evaluator yet) are treated as "always true" so pure matches still fire.
    #[serde(default)]
    pub pred: Option<String>,
}

/// The boolean condition guarding a rule's actions. Composable AND/OR.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct When {
    /// All triggers must hold (logical AND).
    #[serde(default)]
    pub all: Vec<Trigger>,
    /// Any trigger may hold (logical OR).
    #[serde(default)]
    pub any: Vec<Trigger>,
}

/// A single declarative rule: a `when` guard plus the actions it fires.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub when: When,
    pub actions: Vec<Action>,
}

/// The full ruleset loaded from TOML (bootstrap ConfigMap or zenoh hot-reload).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Rules {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Rules {
    /// Parse a ruleset from TOML text. Errors are surfaced to the caller so the
    /// engine can reject bad config and keep the previous ruleset active.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}
