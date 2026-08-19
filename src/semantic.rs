use std::collections::HashMap;

use serde::Deserialize;

use crate::rules::{
    Action, EvalMode, Op, Operand, Predicate, PrimitiveRef, Qos, Rule, Rules, Ruleset, Trigger,
    When,
};

fn default_qos() -> Qos {
    Qos::Reliable
}

// ---------------------------------------------------------------------------
// Structured error type
// ---------------------------------------------------------------------------

/// Error code for a semantic rule error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// TOML parse failure.
    Parse,
    /// Action has no known verb.
    NoActionVerb,
    /// Distance value out of range.
    InvalidDistance,
    /// References a zone not defined in `[zones]`.
    UnknownZone,
    /// Missing required field.
    MissingField,
    /// Payload is not a primitive type.
    NonPrimitivePayload,
    /// Ruleset name is invalid.
    InvalidRulesetName,
    /// Topic does not match naming convention.
    InvalidTopic,
    /// The `when` guard is empty (no condition key, all, or any).
    EmptyWhen,
    /// A nested `when` shape the runtime `When` model cannot express.
    UnrepresentableNesting,
}

impl ErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::Parse => "E001",
            ErrorCode::NoActionVerb => "E002",
            ErrorCode::InvalidDistance => "E003",
            ErrorCode::UnknownZone => "E004",
            ErrorCode::MissingField => "E005",
            ErrorCode::NonPrimitivePayload => "E006",
            ErrorCode::InvalidRulesetName => "E007",
            ErrorCode::InvalidTopic => "E008",
            ErrorCode::EmptyWhen => "E009",
            ErrorCode::UnrepresentableNesting => "E010",
        }
    }
}

/// Structured error for semantic rule parse, validate, and compile operations.
#[derive(Debug, Clone)]
pub struct SemanticError {
    pub message: String,
    pub code: ErrorCode,
    pub field_path: Option<String>,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl SemanticError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        SemanticError {
            message: message.into(),
            code,
            field_path: None,
            line: None,
            col: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }

    pub fn with_span(mut self, line: usize, col: usize) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }
}

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error[{}]: {}", self.code.as_str(), self.message)?;
        if let Some(ref path) = self.field_path {
            write!(f, "\n  --> {path}")?;
        }
        if let (Some(line), Some(col)) = (self.line, self.col) {
            write!(f, " at line {line}:{col}")?;
        }
        Ok(())
    }
}
impl std::error::Error for SemanticError {}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Site {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub frame: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct NearSpec {
    pub entity: String,
    pub dist: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// Parse an extended-TOML semantic document.
pub fn parse_semantic(text: &str) -> Result<SemanticDoc, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

/// Parse a `Ruleset` envelope from extended-TOML.
pub fn parse_semantic_ruleset(text: &str) -> Result<SemanticRuleset, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

/// Attempt JSON parse, fall back to TOML. Detects format from first non-whitespace
/// character (`{` means JSON, anything else means TOML).
pub fn parse_semantic_auto(text: &str) -> Result<SemanticDoc, SemanticError> {
    match guess_format(text) {
        Format::Json => parse_semantic_json(text),
        Format::Toml => parse_semantic(text),
    }
}

/// Attempt JSON parse, fall back to TOML for the Ruleset envelope.
pub fn parse_semantic_ruleset_auto(text: &str) -> Result<SemanticRuleset, SemanticError> {
    match guess_format(text) {
        Format::Json => parse_semantic_ruleset_json(text),
        Format::Toml => parse_semantic_ruleset(text),
    }
}

fn guess_format(text: &str) -> Format {
    if text.trim().starts_with('{') {
        Format::Json
    } else {
        Format::Toml
    }
}

enum Format {
    Json,
    Toml,
}

/// Parse a semantic doc from JSON.
pub fn parse_semantic_json(text: &str) -> Result<SemanticDoc, SemanticError> {
    serde_json::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

/// Parse a Ruleset envelope from JSON.
pub fn parse_semantic_ruleset_json(text: &str) -> Result<SemanticRuleset, SemanticError> {
    serde_json::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

// ---------------------------------------------------------------------------
// Validate (semantic doc)
// ---------------------------------------------------------------------------

/// Validate semantic invariants before compile.
pub fn validate(doc: &SemanticDoc) -> Result<(), SemanticError> {
    for (rule_idx, rule) in doc.rules.iter().enumerate() {
        for (action_idx, a) in rule.actions.iter().enumerate() {
            if !a.estop && a.slow_to.is_none() && !a.resume {
                return Err(SemanticError::new(
                    ErrorCode::NoActionVerb,
                    format!(
                        "rule '{}': action has no known verb (estop/slow_to/resume)",
                        rule.name
                    ),
                )
                .with_path(format!("rules[{rule_idx}].actions[{action_idx}]")));
            }
        }
        validate_when(
            &rule.when,
            &rule.name,
            doc,
            &format!("rules[{rule_idx}].when"),
        )?;
    }
    Ok(())
}

/// A `SemanticWhen` is empty when it carries no flat condition key and no
/// nested `all`/`any` blocks. An empty when would otherwise evaluate
/// vacuously-true and fire every tick.
fn when_is_empty(when: &SemanticWhen) -> bool {
    when.in_zone.is_none()
        && when.not_in_zone.is_none()
        && when.near_human.is_none()
        && when.not_near_human.is_none()
        && when.near.is_none()
        && when.role.is_none()
        && when.all.is_empty()
        && when.any.is_empty()
}

/// Recursively validate a `SemanticWhen` (flat fields plus nested `all`/`any`).
fn validate_when(
    when: &SemanticWhen,
    rule_name: &str,
    doc: &SemanticDoc,
    path: &str,
) -> Result<(), SemanticError> {
    if when_is_empty(when) {
        return Err(SemanticError::new(
            ErrorCode::EmptyWhen,
            format!("rule '{rule_name}': when is empty (no condition key, no all, no any)"),
        )
        .with_path(path));
    }
    for d in [
        when.near_human,
        when.not_near_human,
        when.near.as_ref().map(|n| n.dist),
    ]
    .into_iter()
    .flatten()
    {
        if d <= 0.0 {
            return Err(SemanticError::new(
                ErrorCode::InvalidDistance,
                format!("rule '{rule_name}': distance must be > 0, got {d}"),
            )
            .with_path(path));
        }
    }
    for z in [when.in_zone.clone(), when.not_in_zone.clone()]
        .into_iter()
        .flatten()
    {
        if !doc.zones.contains_key(&z) {
            return Err(SemanticError::new(
                ErrorCode::UnknownZone,
                format!("rule '{rule_name}': references unknown zone '{z}'"),
            )
            .with_path(path));
        }
    }
    for (nested_idx, nested) in when.all.iter().enumerate() {
        validate_when(nested, rule_name, doc, &format!("{path}.all[{nested_idx}]"))?;
    }
    for (nested_idx, nested) in when.any.iter().enumerate() {
        validate_when(nested, rule_name, doc, &format!("{path}.any[{nested_idx}]"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compile (semantic doc → runtime Rules)
// ---------------------------------------------------------------------------

/// Compile a validated semantic doc to the runtime `Rules` shape.
pub fn compile(doc: &SemanticDoc, robot_id: &str) -> Result<Rules, SemanticError> {
    validate(doc)?;
    if doc.site.id.is_empty() {
        return Err(
            SemanticError::new(ErrorCode::MissingField, "missing [site].id").with_path("site.id"),
        );
    }

    let mut out = Vec::new();
    for rule in &doc.rules {
        let (all, any) = expand_when(&rule.when, robot_id, &rule.name, "when")?;

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

// ---------------------------------------------------------------------------
// Expand when → triggers
// ---------------------------------------------------------------------------

/// Recursively expand a `SemanticWhen` into runtime trigger lists.
///
/// The runtime `When` model is two-level: `(AND over all) AND (OR over any,
/// if any is non-empty)`. Nested blocks are flattened into that shape:
///
/// - a nested block in `all` (AND context) contributes its `all` triggers to
///   the parent's `all`, and its `any` group becomes the parent's single `any`
///   group (only one such group is representable — a second would be two OR
///   groups ANDed together, which the runtime cannot express);
/// - a nested block in `any` (OR context) contributes a single trigger, or a
///   pure OR group whose triggers merge into the parent's `any`. A block that
///   is itself an AND of two or more triggers inside an OR element is not
///   representable and is rejected rather than silently mis-flattened.
///
/// Fail-closed: anything the two-level model cannot express returns
/// `ErrorCode::UnrepresentableNesting` instead of silently dropping
/// conditions.
fn expand_when(
    when: &SemanticWhen,
    robot_id: &str,
    rule_name: &str,
    path: &str,
) -> Result<(Vec<Trigger>, Vec<Trigger>), SemanticError> {
    let mut all = Vec::new();
    let mut any = Vec::new();

    if when_is_empty(when) {
        return Err(SemanticError::new(
            ErrorCode::EmptyWhen,
            format!("rule '{rule_name}': when is empty (no condition key, no all, no any)"),
        )
        .with_path(path));
    }

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

    for (nested_idx, nested) in when.all.iter().enumerate() {
        let (nested_all, nested_any) = expand_when(
            nested,
            robot_id,
            rule_name,
            &format!("{path}.all[{nested_idx}]"),
        )?;
        if !nested_any.is_empty() {
            if !any.is_empty() {
                return Err(SemanticError::new(
                    ErrorCode::UnrepresentableNesting,
                    format!(
                        "rule '{rule_name}': {path}.all[{nested_idx}] introduces a second OR \
                         group; the runtime model can hold only one (an AND of two OR groups)"
                    ),
                )
                .with_path(format!("{path}.all[{nested_idx}]")));
            }
            any.extend(nested_any);
        }
        all.extend(nested_all);
    }

    for (nested_idx, nested) in when.any.iter().enumerate() {
        let (nested_all, nested_any) = expand_when(
            nested,
            robot_id,
            rule_name,
            &format!("{path}.any[{nested_idx}]"),
        )?;
        if !nested_all.is_empty() && !nested_any.is_empty() {
            return Err(SemanticError::new(
                ErrorCode::UnrepresentableNesting,
                format!(
                    "rule '{rule_name}': {path}.any[{nested_idx}] is an AND of triggers ANDed \
                     with an OR group, which cannot be expressed as a single OR element"
                ),
            )
            .with_path(format!("{path}.any[{nested_idx}]")));
        }
        if nested_any.is_empty() {
            if nested_all.len() != 1 {
                return Err(SemanticError::new(
                    ErrorCode::UnrepresentableNesting,
                    format!(
                        "rule '{rule_name}': {path}.any[{nested_idx}] is an AND of {} triggers; \
                         an OR element must be a single trigger or an OR group",
                        nested_all.len()
                    ),
                )
                .with_path(format!("{path}.any[{nested_idx}]")));
            }
            any.push(nested_all.into_iter().next().unwrap());
        } else {
            any.extend(nested_any);
        }
    }

    Ok((all, any))
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
// Ruleset envelope path
// ---------------------------------------------------------------------------

/// Envelope-parse shape for a `Ruleset` authored as extended TOML.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRuleset {
    pub ruleset_name: String,
    #[serde(default)]
    pub version: u64,
    pub robot_owner: String,
    #[serde(default, rename = "rule")]
    pub rules: Vec<SemanticRulesetRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRulesetRule {
    pub rule_name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    pub actions: Vec<SemanticRulesetAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRulesetAction {
    pub topic: String,
    #[serde(default = "default_qos")]
    pub qos: Qos,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Compile a `Ruleset` envelope into the runtime `Ruleset` wire/storage unit.
pub fn compile_ruleset(doc: &SemanticRuleset, robot_id: &str) -> Result<Ruleset, SemanticError> {
    let ruleset_name = doc.ruleset_name.to_lowercase();
    if !ruleset_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || ruleset_name.is_empty()
        || ruleset_name.len() > 64
    {
        return Err(SemanticError::new(
            ErrorCode::InvalidRulesetName,
            format!("invalid ruleset_name '{ruleset_name}' (must match [a-z0-9-]{{1,64}})"),
        )
        .with_path("ruleset_name"));
    }

    let mut rules = Vec::new();
    for (rule_idx, rule) in doc.rules.iter().enumerate() {
        let (all, any) = expand_when(&rule.when, robot_id, &rule.rule_name, "when")?;
        let actions: Vec<Action> = rule.actions.iter().map(compile_ruleset_action).collect();
        validate_rule_payloads(&actions, &rule.rule_name, &format!("rules[{rule_idx}]"))?;
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

fn validate_rule_payloads(
    actions: &[Action],
    rule_name: &str,
    path: &str,
) -> Result<(), SemanticError> {
    for (action_idx, a) in actions.iter().enumerate() {
        if !is_primitive(&a.payload) {
            return Err(SemanticError::new(
                ErrorCode::NonPrimitivePayload,
                format!("rule '{rule_name}': action payload must be primitive (bool/int/float/string), got {a:?}"),
            ).with_path(format!("{path}.actions[{action_idx}].payload")));
        }
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_semantic_doc() {
        let json = r#"{
            "site": { "id": "cell-7", "frame": "cell-7/world" },
            "zones": { "safety": { "shape": "rect", "x": 0.0, "y": 0.0, "w": 2.0, "h": 2.0 } },
            "rules": [
                { "name": "hrc-slow-near-human", "when": { "near_human": 1.2 }, "actions": [{ "slow_to": 0.1, "qos": "best_effort" }] }
            ]
        }"#;
        let doc = parse_semantic_json(json).expect("parse JSON doc");
        assert_eq!(doc.site.id, "cell-7");
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].when.near_human, Some(1.2));
    }

    #[test]
    fn parse_json_ruleset() {
        let json = r#"{
            "ruleset_name": "acme-site-a",
            "version": 3,
            "robot_owner": "robot/7",
            "rule": [
                {
                    "rule_name": "slow_near_human",
                    "when": { "in_zone": "zone_1", "near_human": 1.2 },
                    "actions": [{ "topic": "robot/7/local/drive", "qos": "reliable", "payload": { "speed_mps": 0.3 } }]
                }
            ]
        }"#;
        let doc = parse_semantic_ruleset_json(json).expect("parse JSON ruleset");
        assert_eq!(doc.ruleset_name, "acme-site-a");
        assert_eq!(doc.rules.len(), 1);
    }

    #[test]
    fn auto_detects_json_from_brace() {
        let json = r#"{"site":{"id":"x"}}"#;
        let doc = parse_semantic_auto(json).expect("auto-detect JSON");
        assert_eq!(doc.site.id, "x");
    }

    #[test]
    fn auto_detects_toml_from_non_brace() {
        let toml = r#"[site]
id = "x""#;
        let doc = parse_semantic_auto(toml).expect("auto-detect TOML");
        assert_eq!(doc.site.id, "x");
    }

    #[test]
    fn json_compile_roundtrip() {
        let json = r#"{
            "site": { "id": "cell-7", "frame": "cell-7/world" },
            "zones": { "safety": { "shape": "rect", "x": 0.0, "y": 0.0, "w": 2.0, "h": 2.0 } },
            "rules": [
                { "name": "test-rule", "when": { "near_human": 1.5 }, "actions": [{ "slow_to": 0.2 }] }
            ]
        }"#;
        let doc = parse_semantic_json(json).unwrap();
        let rules = compile(&doc, "7").unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].name, "test-rule");
    }

    #[test]
    fn json_parse_fails_on_bad_json() {
        let bad = r#"{"site": {"id":}"#;
        assert!(parse_semantic_json(bad).is_err());
    }

    #[test]
    fn json_ruleset_compile() {
        let json = r#"{
            "ruleset_name": "test-site",
            "version": 1,
            "robot_owner": "robot/7",
            "rule": [
                {
                    "rule_name": "r1",
                    "when": { "in_zone": "safety" },
                    "actions": [{ "topic": "robot/7/local/drive", "payload": { "speed_mps": 0.1 } }]
                }
            ]
        }"#;
        let doc = parse_semantic_ruleset_json(json).unwrap();
        let rs = compile_ruleset(&doc, "7").unwrap();
        assert_eq!(rs.ruleset_name, "test-site");
        assert_eq!(rs.rules[0].name, "r1");
    }
}
