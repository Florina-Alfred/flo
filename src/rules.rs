use serde::{Deserialize, Serialize};

/// QoS class a published action targets. Maps onto the locked transport decision:
/// `reliable` => Zenoh class 1 (STOP), `best_effort` => Zenoh class 2 (lidar).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Qos {
    Reliable,
    BestEffort,
}

/// Boolean/arithmetic operator for a `Predicate` comparison (PRD §4 grammar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    SameZoneAs,
}

/// A comparison operand: a literal or a typed primitive reference (PRD §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Prim(PrimitiveRef),
}

/// One of the five rule primitives (PRD §4). `Proximity` carries the peer robot id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveRef {
    Site,
    Zone,
    Robot,
    Proximity(String),
    HumanPresence,
}

/// Evaluation mode for a trigger (PRD §1 fog, #77): fire on transition (Edge)
/// or re-evaluate every tick against latest sample (Level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EvalMode {
    #[default]
    Edge,
    Level,
}

/// A statically-auditable predicate tree (non-Turing-complete, deterministic).
/// Replaces the legacy free-text `Trigger.pred: Option<String>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Predicate {
    Comparison { op: Op, lhs: Operand, rhs: Operand },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

/// A single publish action fired when a rule's `when` evaluates true.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Action {
    /// Target key-expression, e.g. `stop/fleet/cmd` or `robot/7/local/drive`.
    pub topic: String,
    /// QoS class for the publish.
    pub qos: Qos,
    /// Free-form payload shipped with the publish (serialized as JSON bytes).
    pub payload: serde_json::Value,
}

/// One predicate: a key-expression match plus an optional typed predicate
/// evaluated against the received payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trigger {
    /// Key-expression the incoming sample must match, e.g. `robot/7/local/bumper`.
    pub topic: String,
    /// Typed predicate over the payload (None => always true).
    #[serde(default)]
    pub pred: Option<Predicate>,
    /// Evaluation mode (#77); defaults to Edge.
    #[serde(default)]
    pub mode: EvalMode,
}

/// The boolean condition guarding a rule's actions. Composable AND/OR.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct When {
    /// All triggers must hold (logical AND).
    #[serde(default)]
    pub all: Vec<Trigger>,
    /// Any trigger may hold (logical OR).
    #[serde(default)]
    pub any: Vec<Trigger>,
}

/// A single declarative rule: a `when` guard plus the actions it fires.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub when: When,
    pub actions: Vec<Action>,
}

/// The full ruleset loaded from TOML (bootstrap ConfigMap or zenoh hot-reload).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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

    /// Serialize back to TOML — used to feed `RuleStore::bootstrap` after compile.
    /// `allow(dead_code)`: only the `semantic_rules` example + production compile path
    /// consume this; clippy's per-crate analysis flags it as unused from the lib's view.
    #[allow(dead_code)]
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("Rules is serializable")
    }
}

/// The full ruleset: an ownership/version envelope wrapping the runtime `Rule`s.
/// This is the wire + storage unit; `rules` is what `engine.rs` evaluates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ruleset {
    pub ruleset_name: String,
    pub version: u64,
    pub robot_owner: String,
    pub rules: Vec<Rule>,
}

impl Ruleset {
    #[allow(dead_code)]
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    #[allow(dead_code)]
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("Ruleset is serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predicate_tree_is_typed() {
        let p = Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::Proximity("7".into())),
            rhs: Operand::Float(1.2),
        };
        assert_eq!(p, p.clone());
        // default eval mode is Edge
        assert_eq!(Trigger::default().mode, EvalMode::Edge);
    }
}
