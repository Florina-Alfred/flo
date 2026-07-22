//! Semantic rule-authoring layer: parse extended-TOML and compile to `rules::Rules`.
//! See docs/superpowers/specs/2026-07-19-industrial-rules-design.md.

use std::collections::HashMap;

use serde::Deserialize;

use crate::rules::{
    Action, EvalMode, Op, Operand, Predicate, PrimitiveRef, Qos, Rule, Rules, Ruleset, Trigger,
    When,
};

fn default_qos() -> Qos {
    Qos::Reliable
}

/// Error type for semantic parse/validate/compile.
#[derive(Debug)]
pub struct SemanticError(pub String);

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "semantic rule error: {}", self.0)
    }
}
impl std::error::Error for SemanticError {}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Site {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub frame: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Zone {
    pub shape: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NearSpec {
    pub entity: String,
    pub dist: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SemanticWhen {
    #[serde(default)]
    pub in_zone: Option<String>,
    #[serde(default)]
    pub not_in_zone: Option<String>,
    #[serde(default)]
    pub near_human: Option<f64>,
    #[serde(default)]
    pub not_near_human: Option<f64>,
    #[serde(default)]
    pub near: Option<NearSpec>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub all: Vec<SemanticWhen>,
    #[serde(default)]
    pub any: Vec<SemanticWhen>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticAction {
    #[serde(default)]
    pub estop: bool,
    #[serde(default)]
    pub slow_to: Option<f64>,
    #[serde(default)]
    pub resume: bool,
    #[serde(default = "default_qos")]
    pub qos: Qos,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRule {
    pub name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    pub actions: Vec<SemanticAction>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SemanticDoc {
    #[serde(default)]
    pub site: Site,
    #[serde(default)]
    pub zones: HashMap<String, Zone>,
    #[serde(default)]
    pub rules: Vec<SemanticRule>,
}

/// Parse an extended-TOML semantic document.
pub fn parse_semantic(text: &str) -> Result<SemanticDoc, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError(e.to_string()))
}

/// Validate semantic invariants before compile.
pub fn validate(doc: &SemanticDoc) -> Result<(), SemanticError> {
    for rule in &doc.rules {
        // every action must carry at least one known verb
        for a in &rule.actions {
            if !a.estop && a.slow_to.is_none() && !a.resume {
                return Err(SemanticError(format!(
                    "rule '{}': action has no known verb (estop/slow_to/resume)",
                    rule.name
                )));
            }
        }
        validate_when(&rule.when, &rule.name, doc)?;
    }
    Ok(())
}

/// Recursively validate a `SemanticWhen` (flat fields plus nested `all`/`any`).
fn validate_when(
    when: &SemanticWhen,
    rule_name: &str,
    doc: &SemanticDoc,
) -> Result<(), SemanticError> {
    for d in [
        when.near_human,
        when.not_near_human,
        when.near.as_ref().map(|n| n.dist),
    ]
    .into_iter()
    .flatten()
    {
        if d <= 0.0 {
            return Err(SemanticError(format!(
                "rule '{rule_name}': distance must be > 0, got {d}"
            )));
        }
    }
    for z in [when.in_zone.clone(), when.not_in_zone.clone()]
        .into_iter()
        .flatten()
    {
        if !doc.zones.contains_key(&z) {
            return Err(SemanticError(format!(
                "rule '{rule_name}': references unknown zone '{z}'"
            )));
        }
    }
    for nested in when.all.iter().chain(when.any.iter()) {
        validate_when(nested, rule_name, doc)?;
    }
    Ok(())
}

/// Compile a validated semantic doc to the runtime `Rules` shape.
pub fn compile(doc: &SemanticDoc, robot_id: &str) -> Result<Rules, SemanticError> {
    validate(doc)?;
    if doc.site.id.is_empty() {
        return Err(SemanticError("missing [site].id".to_string()));
    }

    let mut out = Vec::new();
    for rule in &doc.rules {
        let (all, any) = expand_when(&rule.when, robot_id);

        let actions: Vec<Action> = rule
            .actions
            .iter()
            .map(|a| compile_action(a, robot_id))
            .collect();

        out.push(Rule {
            name: rule.name.clone(),
            when: When { all, any },
            actions,
        });
    }
    Ok(Rules { rules: out })
}

/// Recursively expand a `SemanticWhen` into runtime trigger lists.
///
/// Returns `(all, any)` where every trigger in `all` must hold (logical AND)
/// and any trigger in `any` may hold (logical OR).
///
/// Flat fields each contribute to `all` (matching prior flat-only behavior).
/// `when.all` nests further AND-requirements; `when.any` nests OR-branches,
/// each nested `SemanticWhen`'s own `all` triggers becoming an OR alternative.
fn expand_when(when: &SemanticWhen, robot_id: &str) -> (Vec<Trigger>, Vec<Trigger>) {
    let mut all = Vec::new();
    let mut any = Vec::new();

    if let Some(z) = &when.in_zone {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/zone"),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }
    if let Some(z) = &when.not_in_zone {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/zone"),
            pred: Some(Predicate::Not(Box::new(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }))),
            mode: EvalMode::Edge,
        });
    }
    if let Some(d) = when.near_human {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/human_present"),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(d) = when.not_near_human {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/human_present"),
            pred: Some(Predicate::Comparison {
                op: Op::Ge,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(n) = &when.near {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/proximity"),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity(n.entity.clone())),
                rhs: Operand::Float(n.dist),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(r) = &when.role {
        all.push(Trigger {
            topic: format!("robot/{robot_id}/local/role"),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Robot),
                rhs: Operand::Str(r.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }

    // Flatten nested `when.all`/`when.any` into the parent trigger lists rather
    // than wrapping them into a single multi-topic trigger. Each nested
    // `SemanticWhen`'s own triggers keep their own `topic` + `pred`, so the
    // engine evaluates every field against its own payload (fail-closed).
    // Merging into one trigger would evaluate a cross-topic predicate against a
    // single payload, silently passing absent-field exclusions (fail-open).
    for nested in &when.all {
        let (nested_all, _) = expand_when(nested, robot_id);
        all.extend(nested_all);
    }
    for nested in &when.any {
        let (nested_all, _) = expand_when(nested, robot_id);
        any.extend(nested_all);
    }

    (all, any)
}

fn compile_action(a: &SemanticAction, robot_id: &str) -> Action {
    if a.estop {
        Action {
            topic: "stop/fleet/cmd".to_string(),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "stop": true }),
        }
    } else if a.resume {
        Action {
            topic: format!("robot/{robot_id}/local/drive"),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "resume": true }),
        }
    } else {
        Action {
            topic: format!("robot/{robot_id}/local/drive"),
            qos: a.qos,
            payload: serde_json::json!({ "speed_mps": a.slow_to.unwrap_or(0.0) }),
        }
    }
}

// ---------------------------------------------------------------------------
// Ruleset envelope path (additive; does not touch the legacy `compile` above).
// ---------------------------------------------------------------------------

/// Envelope-parse shape for a `Ruleset` authored as extended TOML. Distinct
/// from `SemanticDoc`/`SemanticRule` (the legacy flat `Rules` path) so the two
/// schemas can coexist without name collisions.
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRuleset {
    pub ruleset_name: String,
    #[serde(default)]
    pub version: u64,
    pub robot_owner: String,
    #[serde(default, rename = "rule")]
    pub rules: Vec<SemanticRulesetRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRulesetRule {
    pub rule_name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    pub actions: Vec<SemanticRulesetAction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRulesetAction {
    pub topic: String,
    #[serde(default = "default_qos")]
    pub qos: Qos,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Parse a `Ruleset` envelope from extended-TOML.
pub fn parse_semantic_ruleset(text: &str) -> Result<SemanticRuleset, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError(e.to_string()))
}

/// Compile a `Ruleset` envelope into the runtime `Ruleset` wire/storage unit.
pub fn compile_ruleset(doc: &SemanticRuleset, robot_id: &str) -> Result<Ruleset, SemanticError> {
    // Normalize ruleset_name: lowercase, restricted charset, 1..=64 bytes.
    let ruleset_name = doc.ruleset_name.to_lowercase();
    if !ruleset_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || ruleset_name.is_empty()
        || ruleset_name.len() > 64
    {
        return Err(SemanticError(format!(
            "invalid ruleset_name '{ruleset_name}' (must match [a-z0-9-]{{1,64}})"
        )));
    }

    let mut rules = Vec::new();
    for rule in &doc.rules {
        let (all, any) = expand_when(&rule.when, robot_id);
        let actions: Vec<Action> = rule.actions.iter().map(compile_ruleset_action).collect();
        validate_rule_payloads(&actions, &rule.rule_name)?;
        rules.push(Rule {
            name: rule.rule_name.clone(),
            when: When { all, any },
            actions,
        });
    }

    Ok(Ruleset {
        ruleset_name,
        version: doc.version,
        robot_owner: doc.robot_owner.clone(),
        rules,
    })
}

fn compile_ruleset_action(a: &SemanticRulesetAction) -> Action {
    Action {
        topic: a.topic.clone(),
        qos: a.qos,
        payload: a.payload.clone(),
    }
}

fn validate_rule_payloads(actions: &[Action], rule_name: &str) -> Result<(), SemanticError> {
    for a in actions {
        if !is_primitive(&a.payload) {
            return Err(SemanticError(format!(
                "rule '{rule_name}': action payload must be primitive (bool/int/float/string), got {a:?}"
            )));
        }
    }
    Ok(())
}

/// A payload is primitive-only when it is a leaf (bool/int/float/string) or a
/// flat object whose values are all leaves. Nested objects/arrays are rejected
/// at author time (PRD: primitive-only payloads).
fn is_primitive(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => true,
        serde_json::Value::Object(m) => m.values().all(is_leaf),
        _ => false,
    }
}

fn is_leaf(v: &serde_json::Value) -> bool {
    matches!(
        v,
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) | serde_json::Value::String(_)
    )
}
