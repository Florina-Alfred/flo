//! Semantic rule-authoring layer: parse extended-TOML and compile to `rules::Rules`.
//! See docs/superpowers/specs/2026-07-19-industrial-rules-design.md.

use std::collections::HashMap;

use serde::Deserialize;

use crate::rules::Qos;

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
        // distance must be positive where present
        for d in [
            rule.when.near_human,
            rule.when.not_near_human,
            rule.when.near.as_ref().map(|n| n.dist),
        ]
        .into_iter()
        .flatten()
        {
            if d <= 0.0 {
                return Err(SemanticError(format!(
                    "rule '{}': distance must be > 0, got {d}",
                    rule.name
                )));
            }
        }
        // every action must carry at least one known verb
        for a in &rule.actions {
            if !a.estop && a.slow_to.is_none() && !a.resume {
                return Err(SemanticError(format!(
                    "rule '{}': action has no known verb (estop/slow_to/resume)",
                    rule.name
                )));
            }
        }
        // referenced zone must exist (when uses in_zone/not_in_zone)
        for z in [rule.when.in_zone.clone(), rule.when.not_in_zone.clone()]
            .into_iter()
            .flatten()
        {
            if !doc.zones.contains_key(&z) {
                return Err(SemanticError(format!(
                    "rule '{}': references unknown zone '{z}'",
                    rule.name
                )));
            }
        }
    }
    Ok(())
}
