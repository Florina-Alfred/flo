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

/// A comparison operand: a literal, a typed primitive reference (PRD §4), or a
/// payload field looked up by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A literal string. To read a payload field by name, use `Field` instead.
    Str(String),
    /// Read the named payload field (e.g. `speed_mps`), failing closed when absent.
    Field(String),
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

    /// Serialize back to TOML — used to feed `ActiveRules::bootstrap` after compile.
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
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

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
        // typed serde round-trip must preserve the comparison tree
        let json = serde_json::to_string(&p).expect("serialize predicate");
        let back: Predicate = serde_json::from_str(&json).expect("deserialize predicate");
        assert_eq!(p, back);
        match back {
            Predicate::Comparison { op, lhs, rhs } => {
                assert_eq!(op, Op::Lt);
                assert_eq!(lhs, Operand::Prim(PrimitiveRef::Proximity("7".into())));
                assert_eq!(rhs, Operand::Float(1.2));
            }
            _ => panic!("expected Comparison"),
        }
        // complex tree round-trips through JSON
        let complex = Predicate::And(vec![
            p.clone(),
            Predicate::Or(vec![Predicate::Not(Box::new(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Field("pressed".into()),
                rhs: Operand::Bool(true),
            }))]),
        ]);
        let json2 = serde_json::to_string(&complex).unwrap();
        let back2: Predicate = serde_json::from_str(&json2).unwrap();
        assert_eq!(complex, back2);
        // TOML round-trip via serde_json value (predicate trees are stored as JSON values)
        let val = serde_json::to_value(&p).unwrap();
        let from_val: Predicate = serde_json::from_value(val).unwrap();
        assert_eq!(p, from_val);
        // default eval mode is Edge
        assert_eq!(Trigger::default().mode, EvalMode::Edge);
        // Trigger serde round-trip
        let trigger = Trigger {
            topic: "robot/7/local/bumper".into(),
            pred: Some(p.clone()),
            mode: EvalMode::Level,
        };
        let jt = serde_json::to_string(&trigger).unwrap();
        let bt: Trigger = serde_json::from_str(&jt).unwrap();
        assert_eq!(bt.topic, trigger.topic);
        assert_eq!(bt.mode, trigger.mode);
        assert_eq!(bt.pred, trigger.pred);
        // Action and Rule round-trip
        let action = Action {
            topic: "stop/fleet/cmd".into(),
            qos: Qos::Reliable,
            payload: serde_json::json!({"stop": true}),
        };
        let rule = Rule {
            name: "t-typed".into(),
            when: When {
                all: vec![trigger.clone()],
                any: vec![],
            },
            actions: vec![action.clone()],
        };
        let rules = Rules {
            rules: vec![rule.clone()],
        };
        let toml = rules.to_toml();
        let back_rules = Rules::from_toml(&toml).expect("Rules TOML round-trip");
        assert_eq!(back_rules.rules.len(), 1);
        assert_eq!(back_rules.rules[0].name, "t-typed");
        assert_eq!(back_rules.rules[0].actions[0].topic, "stop/fleet/cmd");
        assert_eq!(back_rules.rules[0].actions[0].qos, Qos::Reliable);
    }
}
