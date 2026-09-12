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
}

impl SemanticError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        SemanticError {
            message: message.into(),
            code,
            field_path: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }
}

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error[{}]: {}", self.code.as_str(), self.message)?;
        if let Some(ref path) = self.field_path {
            write!(f, "\n  --> {path}")?;
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
    /// Raw action form: an explicit topic (and optional payload) instead of the
    /// verb sugar. Shared with the ruleset-envelope path.
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
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
pub struct RulesManifest {
    #[serde(default)]
    pub site: Site,
    #[serde(default)]
    pub zones: HashMap<String, Zone>,
    #[serde(default)]
    pub rules: Vec<SemanticRule>,
}

/// Alias for one release — use [`RulesManifest`] for new code.
pub type SemanticDoc = RulesManifest;

// ---------------------------------------------------------------------------
// WhenExpr IR — can represent arbitrary nesting, then single lower()
// ---------------------------------------------------------------------------

/// Intermediate representation for a `when` guard that can hold arbitrary
/// nesting of AND/OR. A `SemanticWhen` is built into this IR (validating leaf
/// invariants), then `lower(&WhenExpr)` flattens it to the two-level runtime
/// `When { all, any }` with diagnostics.
#[derive(Debug, Clone)]
pub enum WhenExpr {
    All(Vec<WhenExpr>),
    Any(Vec<WhenExpr>),
    Leaf(Trigger),
}

// ---------------------------------------------------------------------------
// ActionSpec IR — replaces slow_to/estop combo
// ---------------------------------------------------------------------------

/// Validated action verb. Replaces the stringly `slow_to: Option<f64>` + `estop: bool` combo.
#[derive(Debug, Clone)]
pub enum ActionSpec {
    Estop,
    SlowTo(f64),
    Resume,
    Raw {
        topic: String,
        payload: serde_json::Value,
    },
}

impl ActionSpec {
    /// Validate a [`SemanticAction`] into a typed verb. The single `TryFrom`-like
    /// entry point replaces the `slow_to.unwrap_or` 4-way branch.
    pub fn try_from_action(a: &SemanticAction, rule_name: &str) -> Result<Self, SemanticError> {
        let mut count = 0u8;
        if a.estop {
            count += 1;
        }
        if a.slow_to.is_some() {
            count += 1;
        }
        if a.resume {
            count += 1;
        }
        if a.topic.is_some() {
            count += 1;
        }
        if count == 0 {
            return Err(SemanticError::new(
                ErrorCode::NoActionVerb,
                format!(
                    "rule '{}': action has no known verb (estop/slow_to/resume)",
                    rule_name
                ),
            ));
        }
        if count > 1 {
            return Err(SemanticError::new(
                ErrorCode::NoActionVerb,
                format!(
                    "rule '{}': action has multiple verbs (only one of estop/slow_to/resume/topic allowed)",
                    rule_name
                ),
            ));
        }
        if let Some(payload) = &a.payload
            && !is_primitive(payload)
        {
            return Err(SemanticError::new(
                ErrorCode::NonPrimitivePayload,
                format!(
                    "rule '{}': action payload must be primitive (bool/int/float/string), got {payload}",
                    rule_name
                ),
            ));
        }
        if a.estop {
            return Ok(ActionSpec::Estop);
        }
        if a.resume {
            return Ok(ActionSpec::Resume);
        }
        if let Some(v) = a.slow_to {
            if !v.is_finite() {
                return Err(SemanticError::new(
                    ErrorCode::InvalidDistance,
                    format!("rule '{}': slow_to must be finite, got {v}", rule_name),
                ));
            }
            return Ok(ActionSpec::SlowTo(v));
        }
        if let Some(topic) = &a.topic {
            if topic.is_empty() {
                return Err(SemanticError::new(
                    ErrorCode::InvalidTopic,
                    format!("rule '{}': action topic is empty", rule_name),
                ));
            }
            let payload = a.payload.clone().unwrap_or(serde_json::Value::Null);
            return Ok(ActionSpec::Raw {
                topic: topic.clone(),
                payload,
            });
        }
        unreachable!()
    }
}

impl TryFrom<&SemanticAction> for ActionSpec {
    type Error = SemanticError;
    fn try_from(a: &SemanticAction) -> Result<Self, Self::Error> {
        // Generic without rule name (used only for external callers that don't have it)
        ActionSpec::try_from_action(a, "<unknown>")
    }
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// Parse an extended-TOML semantic document.
pub fn parse_semantic(text: &str) -> Result<RulesManifest, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

/// Parse a `Ruleset` envelope from extended-TOML.
pub fn parse_semantic_ruleset(text: &str) -> Result<SemanticRuleset, SemanticError> {
    toml::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

/// Attempt JSON parse, fall back to TOML. Detects format from first non-whitespace
/// character (`{` means JSON, anything else means TOML).
pub fn parse_semantic_auto(text: &str) -> Result<RulesManifest, SemanticError> {
    match guess_format(text) {
        Format::Json => parse_semantic_json(text),
        Format::Toml => parse_semantic(text),
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

fn parse_semantic_json(text: &str) -> Result<RulesManifest, SemanticError> {
    serde_json::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

#[cfg(test)]
fn parse_semantic_ruleset_json(text: &str) -> Result<SemanticRuleset, SemanticError> {
    serde_json::from_str(text).map_err(|e| SemanticError::new(ErrorCode::Parse, e.to_string()))
}

// ---------------------------------------------------------------------------
// Validate (semantic doc) — single traversal via WhenExpr builder + ActionSpec
// ---------------------------------------------------------------------------

/// Validate semantic invariants before compile. The single shared validator:
/// both the direct manifest and the ruleset-envelope path (via desugaring)
/// check action verbs, payload primitiveness, `when` shape, distances, and
/// zone references here.
pub fn validate(doc: &RulesManifest) -> Result<(), SemanticError> {
    for (rule_idx, rule) in doc.rules.iter().enumerate() {
        for (action_idx, a) in rule.actions.iter().enumerate() {
            let path = format!("rules[{rule_idx}].actions[{action_idx}]");
            ActionSpec::try_from_action(a, &rule.name)
                .map_err(|e| {
                    if e.field_path.is_none() {
                        e.with_path(&path)
                    } else {
                        // payload errors already have inner path, promote to full
                        if e.field_path.as_deref() == Some("payload") {
                            e.with_path(format!("{path}.payload"))
                        } else {
                            e.with_path(&path)
                        }
                    }
                })
                .map_err(|mut e| {
                    // Ensure payload path is correctly prefixed when needed
                    if e.code == ErrorCode::NonPrimitivePayload
                        && !e.field_path.as_deref().unwrap_or("").ends_with(".payload")
                    {
                        e.field_path = Some(format!("{path}.payload"));
                    }
                    e
                })?;
        }
        // Build WhenExpr with a dummy robot_id to validate leaf invariants
        // (distances, zones, empty) without needing a real robot_id.
        // Do NOT lower — `validate` mirrors the old behavior where
        // UnrepresentableNesting is only caught at compile time.
        let path = format!("rules[{rule_idx}].when");
        let _ = build_when_expr(&rule.when, "validation", doc, &rule.name, &path)?;
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

/// Build a `WhenExpr` IR from a `SemanticWhen`, validating leaf invariants
/// (distances, zones, empty) in the same single traversal that previously was
/// duplicated across `validate_when` and `expand_when`.
fn build_when_expr(
    when: &SemanticWhen,
    robot_id: &str,
    doc: &RulesManifest,
    rule_name: &str,
    path: &str,
) -> Result<WhenExpr, SemanticError> {
    if when_is_empty(when) {
        return Err(SemanticError::new(
            ErrorCode::EmptyWhen,
            format!("rule '{rule_name}': when is empty (no condition key, no all, no any)"),
        )
        .with_path(path));
    }
    // Validate distances
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
    // Validate zones
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

    // Leaf triggers for flat fields
    let mut leaf_triggers = Vec::new();
    if let Some(z) = &when.in_zone {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "zone").into_string(),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }
    if let Some(z) = &when.not_in_zone {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "zone").into_string(),
            pred: Some(Predicate::Not(Box::new(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Zone),
                rhs: Operand::Str(z.clone()),
            }))),
            mode: EvalMode::Edge,
        });
    }
    if let Some(d) = when.near_human {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "human_present").into_string(),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(d) = when.not_near_human {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "human_present").into_string(),
            pred: Some(Predicate::Comparison {
                op: Op::Ge,
                lhs: Operand::Prim(PrimitiveRef::HumanPresence),
                rhs: Operand::Float(d),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(n) = &when.near {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "proximity").into_string(),
            pred: Some(Predicate::Comparison {
                op: Op::Lt,
                lhs: Operand::Prim(PrimitiveRef::Proximity(n.entity.clone())),
                rhs: Operand::Float(n.dist),
            }),
            mode: EvalMode::Level,
        });
    }
    if let Some(r) = &when.role {
        leaf_triggers.push(Trigger {
            topic: crate::topic::robot_local(robot_id, "role").into_string(),
            pred: Some(Predicate::Comparison {
                op: Op::Eq,
                lhs: Operand::Prim(PrimitiveRef::Robot),
                rhs: Operand::Str(r.clone()),
            }),
            mode: EvalMode::Edge,
        });
    }

    // Recursively build children
    let mut all_children = Vec::new();
    for (idx, nested) in when.all.iter().enumerate() {
        let child = build_when_expr(
            nested,
            robot_id,
            doc,
            rule_name,
            &format!("{path}.all[{idx}]"),
        )?;
        all_children.push(child);
    }
    let mut any_children = Vec::new();
    for (idx, nested) in when.any.iter().enumerate() {
        let child = build_when_expr(
            nested,
            robot_id,
            doc,
            rule_name,
            &format!("{path}.any[{idx}]"),
        )?;
        any_children.push(child);
    }

    let mut and_parts: Vec<WhenExpr> = leaf_triggers.into_iter().map(WhenExpr::Leaf).collect();
    and_parts.extend(all_children);
    let any_part = if any_children.is_empty() {
        None
    } else {
        Some(WhenExpr::Any(any_children))
    };

    match (and_parts.is_empty(), any_part) {
        (true, None) => unreachable!("empty already handled"),
        (true, Some(any)) => Ok(any),
        (false, None) => {
            if and_parts.len() == 1 {
                Ok(and_parts.into_iter().next().unwrap())
            } else {
                Ok(WhenExpr::All(and_parts))
            }
        }
        (false, Some(any)) => {
            let mut combined = and_parts;
            combined.push(any);
            Ok(WhenExpr::All(combined))
        }
    }
}

/// Single `lower(&WhenExpr) -> Result<When>` with diagnostics.
/// Flattens arbitrary nesting to the two-level runtime `When` model.
/// Deleting the former `validate`/`expand_when` dual recursion.
fn lower(expr: &WhenExpr, rule_name: &str, path: &str) -> Result<When, SemanticError> {
    match expr {
        WhenExpr::Leaf(t) => Ok(When {
            all: vec![t.clone()],
            any: Vec::new(),
        }),
        WhenExpr::All(children) => {
            let mut all = Vec::new();
            let mut any = Vec::new();
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{path}.all[{idx}]");
                let w = lower(child, rule_name, &child_path)?;
                if !w.any.is_empty() {
                    if !any.is_empty() {
                        return Err(SemanticError::new(
                            ErrorCode::UnrepresentableNesting,
                            format!(
                                "rule '{rule_name}': {path}.all[{idx}] introduces a second OR \
                                 group; the runtime model can hold only one (an AND of two OR groups)"
                            ),
                        )
                        .with_path(child_path));
                    }
                    any = w.any;
                }
                all.extend(w.all);
            }
            Ok(When { all, any })
        }
        WhenExpr::Any(children) => {
            let mut any = Vec::new();
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{path}.any[{idx}]");
                let w = lower(child, rule_name, &child_path)?;
                if !w.all.is_empty() && !w.any.is_empty() {
                    return Err(SemanticError::new(
                        ErrorCode::UnrepresentableNesting,
                        format!(
                            "rule '{rule_name}': {child_path} is an AND of triggers ANDed \
                             with an OR group, which cannot be expressed as a single OR element"
                        ),
                    )
                    .with_path(child_path));
                }
                if w.any.is_empty() {
                    if w.all.len() != 1 {
                        return Err(SemanticError::new(
                            ErrorCode::UnrepresentableNesting,
                            format!(
                                "rule '{rule_name}': {child_path} is an AND of {} triggers; \
                                 an OR element must be a single trigger or an OR group",
                                w.all.len()
                            ),
                        )
                        .with_path(child_path));
                    }
                    any.push(w.all.into_iter().next().unwrap());
                } else {
                    any.extend(w.any);
                }
            }
            Ok(When {
                all: Vec::new(),
                any,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Compile (semantic doc → runtime Rules)
// ---------------------------------------------------------------------------

/// Compile a validated manifest to the runtime `Rules` shape.
pub fn compile(doc: &RulesManifest, robot_id: &str) -> Result<Rules, SemanticError> {
    validate(doc)?;
    if doc.site.id.is_empty() {
        return Err(
            SemanticError::new(ErrorCode::MissingField, "missing [site].id").with_path("site.id"),
        );
    }

    let mut out = Vec::new();
    for (rule_idx, rule) in doc.rules.iter().enumerate() {
        let when_path = format!("rules[{rule_idx}].when");
        let expr = build_when_expr(&rule.when, robot_id, doc, &rule.name, &when_path)?;
        let when = lower(&expr, &rule.name, &when_path)?;

        let actions: Vec<Action> = rule
            .actions
            .iter()
            .enumerate()
            .map(|(action_idx, a)| {
                let spec = ActionSpec::try_from_action(a, &rule.name).map_err(|e| {
                    let base = format!("rules[{rule_idx}].actions[{action_idx}]");
                    if e.field_path.is_none() {
                        e.with_path(&base)
                    } else if e.field_path.as_deref() == Some("payload") {
                        e.with_path(format!("{base}.payload"))
                    } else {
                        e.with_path(&base)
                    }
                })?;
                compile_action_from_spec(&spec, a, robot_id)
            })
            .collect::<Result<Vec<_>, SemanticError>>()?;

        out.push(Rule {
            name: rule.name.clone(),
            when,
            actions,
        });
    }
    Ok(Rules { rules: out })
}

fn compile_action_from_spec(
    spec: &ActionSpec,
    original: &SemanticAction,
    robot_id: &str,
) -> Result<Action, SemanticError> {
    Ok(match spec {
        ActionSpec::Estop => Action {
            topic: crate::topic::stop_cmd("fleet").into_string(),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "stop": true }),
        },
        ActionSpec::Resume => Action {
            topic: crate::topic::robot_local(robot_id, "drive").into_string(),
            qos: Qos::Reliable,
            payload: serde_json::json!({ "resume": true }),
        },
        ActionSpec::SlowTo(v) => Action {
            topic: crate::topic::robot_local(robot_id, "drive").into_string(),
            qos: original.qos,
            payload: serde_json::json!({ "speed_mps": *v }),
        },
        ActionSpec::Raw { topic, payload } => {
            // Validate raw topic strings at construction — a typo fails here, not at publish (typed mesh).
            let validated = crate::topic::Topic::try_new(topic).map_err(|e| {
                SemanticError::new(
                    ErrorCode::InvalidTopic,
                    format!("invalid topic '{topic}': {e}"),
                )
            })?;
            Action {
                topic: validated.into_string(),
                qos: original.qos,
                payload: payload.clone(),
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Ruleset envelope path
// ---------------------------------------------------------------------------

/// Envelope-parse shape for a `Ruleset` authored as extended TOML. Carries the
/// same `site`/`zones`/`when` vocabulary as [`RulesManifest`] plus ownership
/// metadata; validation and compilation are delegated to the shared
/// [`validate`]/[`compile`] through a thin desugaring.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRuleset {
    pub ruleset_name: String,
    #[serde(default)]
    pub version: u64,
    pub robot_owner: String,
    #[serde(default)]
    pub site: Site,
    #[serde(default)]
    pub zones: HashMap<String, Zone>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<SemanticRulesetRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRulesetRule {
    pub rule_name: String,
    #[serde(default)]
    pub when: SemanticWhen,
    #[serde(default)]
    pub actions: Vec<SemanticAction>,
}

/// Compile a `Ruleset` envelope into the runtime `Ruleset` wire/storage unit.
/// A thin wrapper: it validates the envelope's `ruleset_name`, desugars into a
/// [`RulesManifest`], and reuses the single shared validator + compiler.
pub fn compile_ruleset(doc: &SemanticRuleset, robot_id: &str) -> Result<Ruleset, SemanticError> {
    let ruleset_name = normalize_ruleset_name(&doc.ruleset_name)?;
    let semantic = RulesManifest {
        site: doc.site.clone(),
        zones: doc.zones.clone(),
        rules: doc
            .rules
            .iter()
            .map(|r| SemanticRule {
                name: r.rule_name.clone(),
                when: r.when.clone(),
                actions: r.actions.clone(),
            })
            .collect(),
    };
    let rules = compile(&semantic, robot_id)?;
    Ok(Ruleset {
        ruleset_name,
        version: doc.version,
        robot_owner: doc.robot_owner.clone(),
        rules: rules.rules,
    })
}

fn normalize_ruleset_name(name: &str) -> Result<String, SemanticError> {
    let ruleset_name = name.to_lowercase();
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
    Ok(ruleset_name)
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
            "site": { "id": "cell-7" },
            "zones": { "safety": { "shape": "rect", "x": 0.0, "y": 0.0, "w": 2.0, "h": 2.0 } },
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
