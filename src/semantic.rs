//! Semantic rule-authoring layer: parse extended-TOML and compile to `rules::Rules`.
//! See docs/superpowers/specs/2026-07-19-industrial-rules-design.md.

use std::collections::HashMap;

use serde::Deserialize;

use crate::rules::{Action, Qos, Rule, Rules, Trigger, When};

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
    let site = if doc.site.id.is_empty() {
        return Err(SemanticError("missing [site].id".to_string()));
    } else {
        &doc.site.id
    };

    let mut out = Vec::new();
    for rule in &doc.rules {
        let (all, any) = expand_when(&rule.when, site, robot_id);

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
fn expand_when(when: &SemanticWhen, site: &str, robot_id: &str) -> (Vec<Trigger>, Vec<Trigger>) {
    let mut all = Vec::new();
    let mut any = Vec::new();

    if let Some(z) = &when.in_zone {
        all.push(Trigger {
            topic: format!("fleet/{site}/{robot_id}/state"),
            pred: Some(format!("zone_id == \"{z}\"")),
        });
    }
    if let Some(z) = &when.not_in_zone {
        all.push(Trigger {
            topic: format!("fleet/{site}/{robot_id}/state"),
            pred: Some(format!("zone_id != \"{z}\"")),
        });
    }
    if let Some(d) = when.near_human {
        all.push(Trigger {
            topic: format!("fleet/{site}/proximity/{robot_id}/human"),
            pred: Some(format!("separation_distance < {d}")),
        });
    }
    if let Some(d) = when.not_near_human {
        all.push(Trigger {
            topic: format!("fleet/{site}/proximity/{robot_id}/human"),
            pred: Some(format!("separation_distance >= {d}")),
        });
    }
    if let Some(n) = &when.near {
        all.push(Trigger {
            topic: format!("fleet/{site}/{robot_id}/nearest_peer"),
            pred: Some(format!("separation_distance < {}", n.dist)),
        });
    }
    if let Some(r) = &when.role {
        all.push(Trigger {
            topic: format!("fleet/{site}/{robot_id}/state"),
            pred: Some(format!("role == \"{r}\"")),
        });
    }

    for nested in &when.all {
        let (nested_all, _nested_any) = expand_when(nested, site, robot_id);
        all.extend(nested_all);
    }
    for nested in &when.any {
        let (nested_all, _nested_any) = expand_when(nested, site, robot_id);
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
