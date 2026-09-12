//! Topic naming convention — the single owner of the flo topic contract.
//!
//! Every topic the system publishes or subscribes to is named here: fixed
//! topics are `const`s, parameterized topics are builder functions, and the
//! subscription wildcards are pattern constants. No other module constructs
//! topic strings — callers build them through `topic::`, and this module's
//! tests verify every builder's output via [`Topic::try_new`] so the
//! convention cannot drift from what the system actually emits.
//!
//! Convention:
//!
//! | Category           | Pattern                                      |
//! |--------------------|----------------------------------------------|
//! | Robot-local        | `robot/{id}/local/{resource}`                |
//! | Robot location     | `robot/{id}/location/{axis}`                 |
//! | Robot zone         | `robot/{id}/zone`                            |
//! | Robot site         | `robot/{id}/site`                            |
//! | Robot liveliness   | `robot/{id}/client/liveliness`               |
//! | Robot rules        | `robot/{id}/local/rules`                     |
//! | Signaling          | `robot/{id}/signal/presence`                |
//! | Signaling          | `robot/{id}/signal/{peer}/{msgtype}`        |
//! | Camera             | `robot/{id}/local/cam{index}`                |
//! | Fleet registration | `fleet/registration`                         |
//! | Fleet dereg        | `fleet/deregistration`                       |
//! | Fleet alerts       | `fleet/alerts/heartbeat/{robot_id}`          |
//! | Fleet rulesets     | `fleet/{site}/ruleset/{name}`                |
//! | Safety stop        | `stop/{scope}/cmd`                           |
//! | LiDAR              | `lidar/{scope}/scan`                         |
//! | Zone events        | `zone/{zone_id}/entered` / `cleared`         |
//! | Zone events (5-seg)| `zone/{site}/{cell}/{robot_id}/entered` / `cleared` |
//!
//! Zone-event verbs are `entered`/`cleared` everywhere: the engine's zone
//! subscriptions, the client config defaults, and this validator agree. The
//! `enter`/`exit` forms are rejected.
//!
//! `Topic` is the validated newtype — construction *is* validation.
//! `AclExpr` is the validated ACL key-expression (leading slash, wildcard).
//! `Pattern` is the validated subscription pattern (allows `*`/`**`).

use std::fmt;
use std::ops::Deref;

// ---------------------------------------------------------------------------
// Validated newtypes
// ---------------------------------------------------------------------------

/// A validated, canonical topic string. Construction is validation:
/// `Topic::try_new` canonicalizes `robot-7` → `robot/7` and checks the
/// single match table. Builders return `Topic`, so a typo fails at
/// construction, not at publish.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Topic(String);

/// Validated ACL key-expression (e.g. `/robot/{id}/**`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AclExpr(String);

/// Validated subscription pattern (allows `*` / `**` wildcards).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pattern(String);

impl Topic {
    /// Validate and canonicalize `s` into a `Topic`.
    ///
    /// Canonicalizes the legacy hyphen form `robot-7` → `robot/7` at entry,
    /// then checks the single match table `[(prefix, arity, verb)]`.
    pub fn try_new(s: &str) -> Result<Self, TopicError> {
        if s.is_empty() {
            return Err(TopicError {
                topic: s.to_string(),
                kind: TopicErrorKind::Empty,
            });
        }
        let canon = canonicalize(s);
        let parts: Vec<&str> = canon.split('/').collect();
        if matches_one_of(&parts) {
            Ok(Self(canon))
        } else {
            Err(TopicError {
                topic: s.to_string(),
                kind: TopicErrorKind::UnknownPattern,
            })
        }
    }

    /// Borrow as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into `String`.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AclExpr {
    /// Validate an ACL expression like `/robot/{id}/**`.
    pub fn try_new(s: &str) -> Result<Self, TopicError> {
        if s.is_empty() {
            return Err(TopicError {
                topic: s.to_string(),
                kind: TopicErrorKind::Empty,
            });
        }
        // ACL expr must start with '/' and end with '/**' and contain /robot/
        if s.starts_with("/robot/") && s.ends_with("/**") {
            // extract id between /robot/ and /**
            let inner = &s["/robot/".len()..s.len() - "/**".len()];
            if !inner.is_empty() && !inner.contains('/') && !inner.contains('*') {
                return Ok(Self(s.to_string()));
            }
        }
        Err(TopicError {
            topic: s.to_string(),
            kind: TopicErrorKind::UnknownPattern,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Pattern {
    /// Validate a subscription pattern (allows `*` and `**`).
    pub fn try_new(s: &str) -> Result<Self, TopicError> {
        if s.is_empty() {
            return Err(TopicError {
                topic: s.to_string(),
                kind: TopicErrorKind::Empty,
            });
        }
        if s.contains("//") || s.starts_with('/') || s.ends_with('/') {
            // patterns are relative, no leading slash except ACL; no empty segments
            // but allow ** at end like `fleet/*/ruleset/**` which ends with **
            // that is okay if we treat separately
            if s != "fleet/*/ruleset/**" && s.contains("//") {
                return Err(TopicError {
                    topic: s.to_string(),
                    kind: TopicErrorKind::UnknownPattern,
                });
            }
        }
        // Basic check: pattern should be known or match wildcard table
        let canon = canonicalize(s);
        let parts: Vec<&str> = canon.split('/').collect();
        // Allow patterns with * or ** in id positions
        if is_valid_pattern(&parts) {
            Ok(Self(canon))
        } else {
            Err(TopicError {
                topic: s.to_string(),
                kind: TopicErrorKind::UnknownPattern,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Deref for Topic {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl Deref for AclExpr {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl Deref for Pattern {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Topic {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for AclExpr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Pattern {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl fmt::Display for AclExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl From<Topic> for String {
    fn from(t: Topic) -> Self {
        t.0
    }
}
impl From<AclExpr> for String {
    fn from(a: AclExpr) -> Self {
        a.0
    }
}
impl From<Pattern> for String {
    fn from(p: Pattern) -> Self {
        p.0
    }
}

// ---------------------------------------------------------------------------
// Canonicalization + validation table
// ---------------------------------------------------------------------------

/// Canonicalize legacy `robot-7` prefix to `robot/7` at entry.
/// Only the first segment is rewritten; `robot/my-robot/...` is not changed.
fn canonicalize(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("robot-") {
        if let Some(slash) = rest.find('/') {
            let id = &rest[..slash];
            let suffix = &rest[slash..];
            format!("robot/{id}{suffix}")
        } else {
            // No slash: the whole string is `robot-{id}` with no further segments.
            // This is not a valid topic on its own (needs more), but canonicalize anyway.
            format!("robot/{}", rest)
        }
    } else {
        s.to_string()
    }
}

/// Single match table `[(prefix, arity, verb)]` after canonicalization.
/// No duplicated `robot/{id}` vs `robot-{id}` arms — hyphen is already canonicalized.
fn matches_one_of(parts: &[&str]) -> bool {
    match parts {
        // --- Robot-local slash form only (4 or 5 segments) ---
        ["robot", _, "local", _] => parts.len() == 4,
        ["robot", _, "location", _] => parts.len() == 4,
        ["robot", _, "signal", "presence"] => parts.len() == 4,
        ["robot", _, "signal", _, _] => parts.len() == 5,
        ["robot", _, "zone"] => parts.len() == 3,
        ["robot", _, "site"] => parts.len() == 3,
        ["robot", _, "client", "liveliness"] => parts.len() == 4,
        // --- Fleet ---
        ["fleet", "registration"] => parts.len() == 2,
        ["fleet", "deregistration"] => parts.len() == 2,
        ["fleet", "registration", "response", _] => parts.len() == 4,
        ["fleet", "deregistration", "response", _] => parts.len() == 4,
        ["fleet", "alerts", "heartbeat", _] => parts.len() == 4,
        ["fleet", _, "ruleset", _] => parts.len() == 4,
        // --- Safety / sensor ---
        ["stop", _, "cmd"] => parts.len() == 3,
        ["lidar", _, "scan"] => parts.len() == 3,
        // --- Zone events ---
        ["zone", _, "entered"] | ["zone", _, "cleared"] => parts.len() == 3,
        ["zone", _, _, _, "entered" | "cleared"] => parts.len() == 5,
        // --- Test helpers: generic sensor/actuator topics used by core_loop/safety_infra06 ---
        ["sensor", _] => parts.len() == 2,
        ["actuator", _] => parts.len() == 2,
        _ => false,
    }
}

/// Validate a subscription pattern (allows `*`/`**` wildcards).
fn is_valid_pattern(parts: &[&str]) -> bool {
    // Allow `*` in id positions and `**` suffix.
    // We check by replacing * with dummy and seeing if it would be a valid topic,
    // or for **, check prefix.
    match parts {
        // liveliness / presence wildcards
        ["robot", "*", "client", "liveliness"] => true,
        ["robot", "*", "signal", "presence"] => true,
        ["robot", "*", "signal", _, "offer"]
        | ["robot", "*", "signal", _, "answer"]
        | ["robot", "*", "signal", _, "ice"] => true,
        // fleet ruleset wildcard
        ["fleet", "*", "ruleset", "**"] => true,
        // zone patterns
        ["zone", "*", "entered"] | ["zone", "*", "cleared"] => true,
        ["zone", "*", "*", "*", "entered" | "cleared"] => true,
        // concrete topics are also valid patterns (exact match)
        _ => {
            // If pattern contains no wildcards, it must be a valid topic
            if parts.iter().any(|p| *p == "*" || *p == "**") {
                false
            } else {
                matches_one_of(parts)
            }
        }
    }
}

// Keep `check_topic_pattern` as a thin wrapper for backward compatibility.
// New code should use `Topic::try_new`.
pub fn check_topic_pattern(topic: &str) -> Result<(), TopicError> {
    Topic::try_new(topic).map(|_| ())
}

// --- Robot-local topics -----------------------------------------------------

/// A robot-local resource topic: `robot/{id}/local/{resource}`.
pub fn robot_local(robot_id: &str, resource: &str) -> Topic {
    Topic::try_new(&format!("robot/{robot_id}/local/{resource}"))
        .expect("robot_local produced invalid topic")
}

/// Robot ruleset hot-reload key: `robot/{id}/local/rules`.
pub fn rules_key(robot_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{robot_id}/local/rules"))
        .expect("rules_key produced invalid topic")
}

// --- Liveliness -------------------------------------------------------------

/// Per-robot liveliness token topic: `robot/{id}/client/liveliness`.
pub fn liveliness_key(robot_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{robot_id}/client/liveliness"))
        .expect("liveliness_key produced invalid topic")
}

/// Liveliness subscription pattern (all clients' tokens).
pub const LIVELINESS_PATTERN: &str = "robot/*/client/liveliness";

/// Typed accessor for the liveliness pattern.
pub fn liveliness_pattern() -> Pattern {
    Pattern::try_new(LIVELINESS_PATTERN).expect("const pattern valid")
}

// --- Signaling (class-3 video) ----------------------------------------------

/// Presence advertisement topic: `robot/{id}/signal/presence`.
pub fn signal_presence_key(robot_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{robot_id}/signal/presence"))
        .expect("signal_presence_key invalid")
}

/// Presence subscription pattern (any robot's advertisement).
pub const SIGNAL_PRESENCE_PATTERN: &str = "robot/*/signal/presence";

pub fn signal_presence_pattern() -> Pattern {
    Pattern::try_new(SIGNAL_PRESENCE_PATTERN).expect("const pattern valid")
}

/// Offer topic addressed to `peer_id`: `robot/{self}/signal/{peer}/offer`.
pub fn signal_offer_key(self_id: &str, peer_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{self_id}/signal/{peer_id}/offer"))
        .expect("signal_offer_key invalid")
}

/// Answer topic addressed to `peer_id`: `robot/{self}/signal/{peer}/answer`.
pub fn signal_answer_key(self_id: &str, peer_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{self_id}/signal/{peer_id}/answer"))
        .expect("signal_answer_key invalid")
}

/// Trickled-ICE topic addressed to `peer_id`: `robot/{self}/signal/{peer}/ice`.
pub fn signal_ice_key(self_id: &str, peer_id: &str) -> Topic {
    Topic::try_new(&format!("robot/{self_id}/signal/{peer_id}/ice"))
        .expect("signal_ice_key invalid")
}

/// Offer subscription pattern: offers from any peer addressed to `self_id`.
pub fn signal_offer_pattern(self_id: &str) -> Pattern {
    Pattern::try_new(&format!("robot/*/signal/{self_id}/offer"))
        .expect("signal_offer_pattern invalid")
}

/// Answer subscription pattern: answers from any peer addressed to `self_id`.
pub fn signal_answer_pattern(self_id: &str) -> Pattern {
    Pattern::try_new(&format!("robot/*/signal/{self_id}/answer"))
        .expect("signal_answer_pattern invalid")
}

/// ICE subscription pattern: candidates from any peer addressed to `self_id`.
pub fn signal_ice_pattern(self_id: &str) -> Pattern {
    Pattern::try_new(&format!("robot/*/signal/{self_id}/ice")).expect("signal_ice_pattern invalid")
}

// --- Fleet registration -----------------------------------------------------

/// Client registration request topic.
pub const REGISTRATION_KEY: &str = "fleet/registration";

/// Client deregistration request topic.
pub const DEREGISTRATION_KEY: &str = "fleet/deregistration";

/// Fleet heartbeat-alert namespace (per-robot topics below).
pub const HEARTBEAT_ALERTS_KEY: &str = "fleet/alerts/heartbeat";

/// Registration response topic for `robot_id`:
/// `fleet/registration/response/{robot_id}`.
pub fn registration_response(robot_id: &str) -> Topic {
    Topic::try_new(&format!("fleet/registration/response/{robot_id}"))
        .expect("registration_response invalid")
}

/// Deregistration response topic for `robot_id`:
/// `fleet/deregistration/response/{robot_id}`.
pub fn deregistration_response(robot_id: &str) -> Topic {
    Topic::try_new(&format!("fleet/deregistration/response/{robot_id}"))
        .expect("deregistration_response invalid")
}

/// Heartbeat-alert topic for `robot_id`: `fleet/alerts/heartbeat/{robot_id}`.
pub fn heartbeat_alert(robot_id: &str) -> Topic {
    Topic::try_new(&format!("fleet/alerts/heartbeat/{robot_id}")).expect("heartbeat_alert invalid")
}

// --- Fleet rulesets ---------------------------------------------------------

/// Fleet-scoped ruleset publish key: `fleet/{site}/ruleset/{name}`.
pub fn ruleset_pub_key(site: &str, name: &str) -> Topic {
    Topic::try_new(&format!("fleet/{site}/ruleset/{name}")).expect("ruleset_pub_key invalid")
}

/// Ruleset subscription pattern (any site, any name).
pub const RULESET_PUB_PATTERN: &str = "fleet/*/ruleset/**";

pub fn ruleset_pub_pattern() -> Pattern {
    Pattern::try_new(RULESET_PUB_PATTERN).expect("const pattern valid")
}

// --- Safety / sensors -------------------------------------------------------

/// Safety stop command topic: `stop/{scope}/cmd` (QoS class 1).
pub fn stop_cmd(scope: &str) -> Topic {
    Topic::try_new(&format!("stop/{scope}/cmd")).expect("stop_cmd invalid")
}

/// LiDAR scan topic: `lidar/{scope}/scan` (QoS class 2).
pub fn lidar_scan(scope: &str) -> Topic {
    Topic::try_new(&format!("lidar/{scope}/scan")).expect("lidar_scan invalid")
}

// --- Zone events ------------------------------------------------------------

/// Zone-entered event topic: `zone/{zone_id}/entered`.
pub fn zone_entered(zone_id: &str) -> Topic {
    Topic::try_new(&format!("zone/{zone_id}/entered")).expect("zone_entered invalid")
}

/// Zone-cleared event topic: `zone/{zone_id}/cleared`.
pub fn zone_cleared(zone_id: &str) -> Topic {
    Topic::try_new(&format!("zone/{zone_id}/cleared")).expect("zone_cleared invalid")
}

/// Zone-entered subscription pattern (any zone).
pub const ZONE_ENTERED_PATTERN: &str = "zone/*/entered";

pub fn zone_entered_pattern() -> Pattern {
    Pattern::try_new(ZONE_ENTERED_PATTERN).expect("const pattern valid")
}

/// Zone-cleared subscription pattern (any zone).
pub const ZONE_CLEARED_PATTERN: &str = "zone/*/cleared";

pub fn zone_cleared_pattern() -> Pattern {
    Pattern::try_new(ZONE_CLEARED_PATTERN).expect("const pattern valid")
}

// --- ACL namespaces ---------------------------------------------------------

/// Robot-scoped Zenoh ACL key-expression prefix: `/robot/{id}/**`. This is an
/// access-control key expression (leading slash, wildcard suffix), not a
/// concrete topic, so it is not covered by [`Topic::try_new`].
pub fn robot_namespace(robot_id: &str) -> AclExpr {
    AclExpr::try_new(&format!("/robot/{robot_id}/**")).expect("robot_namespace invalid")
}

/// A topic validation error.
#[derive(Debug, Clone)]
pub struct TopicError {
    pub topic: String,
    pub kind: TopicErrorKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopicErrorKind {
    /// Topic string is empty.
    Empty,
    /// Topic does not match any known naming convention pattern.
    UnknownPattern,
}

impl std::fmt::Display for TopicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            TopicErrorKind::Empty => write!(f, "topic is empty"),
            TopicErrorKind::UnknownPattern => {
                write!(
                    f,
                    "topic '{}' does not match the flo naming convention \
                     (expected robot/{{id}}/local/{{resource}}, fleet/{{action}}, \
                     stop/{{scope}}/cmd, lidar/{{scope}}/scan, \
                     zone/{{zone_id}}/entered|cleared, or robot/{{id}}/signal/{{peer}}/{{msgtype}})",
                    self.topic
                )
            }
        }
    }
}
impl std::error::Error for TopicError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_robot_local() {
        assert!(Topic::try_new("robot/7/local/bumper").is_ok());
        assert!(Topic::try_new("robot/my-robot/local/drive").is_ok());
        assert!(Topic::try_new("robot/7/local/human_present").is_ok());
    }

    #[test]
    fn rejects_too_many_local_segments() {
        assert!(Topic::try_new("robot/7/local/bumper/extra").is_err());
    }

    #[test]
    fn valid_robot_location() {
        assert!(Topic::try_new("robot/7/location/x").is_ok());
        assert!(Topic::try_new("robot/7/location/y").is_ok());
        assert!(Topic::try_new("robot/7/location/z").is_ok());
    }

    #[test]
    fn valid_robot_zone_site() {
        assert!(Topic::try_new("robot/7/zone").is_ok());
        assert!(Topic::try_new("robot/7/site").is_ok());
    }

    #[test]
    fn valid_fleet_topics() {
        assert!(Topic::try_new("fleet/registration").is_ok());
        assert!(Topic::try_new("fleet/deregistration").is_ok());
        assert!(Topic::try_new("fleet/alerts/heartbeat/robot7").is_ok());
    }

    #[test]
    fn valid_fleet_response_topics() {
        assert!(Topic::try_new("fleet/registration/response/robot-7").is_ok());
        assert!(Topic::try_new("fleet/deregistration/response/robot-7").is_ok());
    }

    #[test]
    fn valid_fleet_ruleset() {
        assert!(Topic::try_new("fleet/cell-7/ruleset/acme").is_ok());
    }

    #[test]
    fn valid_stop_and_lidar() {
        assert!(Topic::try_new("stop/fleet/cmd").is_ok());
        assert!(Topic::try_new("lidar/fleet/scan").is_ok());
    }

    #[test]
    fn valid_zone_events() {
        assert!(Topic::try_new("zone/cell-3/entered").is_ok());
        assert!(Topic::try_new("zone/cell-3/cleared").is_ok());
        assert!(Topic::try_new("zone/site/cell/7/entered").is_ok());
        assert!(Topic::try_new("zone/site/cell/7/cleared").is_ok());
    }

    #[test]
    fn rejects_non_canonical_zone_events() {
        assert!(Topic::try_new("zone/site/cell/7/enter").is_err());
        assert!(Topic::try_new("zone/site/cell/7/exit").is_err());
        assert!(Topic::try_new("zone/cell-3/7/enter").is_err());
        assert!(Topic::try_new("zone/cell-3/7/exit").is_err());
    }

    #[test]
    fn valid_signal() {
        assert!(Topic::try_new("robot/self/signal/presence").is_ok());
        assert!(Topic::try_new("robot/self/signal/peer/offer").is_ok());
        assert!(Topic::try_new("robot/self/signal/peer/answer").is_ok());
        assert!(Topic::try_new("robot/self/signal/peer/ice").is_ok());
    }

    #[test]
    fn valid_hyphenated_robot_id() {
        // Hyphen prefix canonicalizes to slash: robot-7/local/bumper -> robot/7/local/bumper
        assert!(Topic::try_new("robot-7/local/bumper").is_ok());
        assert_eq!(
            Topic::try_new("robot-7/local/bumper").unwrap().as_str(),
            "robot/7/local/bumper"
        );
        assert!(Topic::try_new("robot-7/zone").is_ok());
        assert_eq!(
            Topic::try_new("robot-7/zone").unwrap().as_str(),
            "robot/7/zone"
        );
        // liveliness hyphen
        assert!(Topic::try_new("robot-7/client/liveliness").is_ok());
        assert_eq!(
            Topic::try_new("robot-7/client/liveliness")
                .unwrap()
                .as_str(),
            "robot/7/client/liveliness"
        );
    }

    #[test]
    fn builders_produce_valid_topics() {
        // Every builder must produce a valid Topic (construction is validation)
        let topics = [
            robot_local("7", "bumper"),
            robot_local("7", "cam0"),
            robot_local("robot-7", "bumper"),
            rules_key("7"),
            liveliness_key("7"),
            signal_presence_key("7"),
            signal_offer_key("7", "9"),
            signal_answer_key("7", "9"),
            signal_ice_key("7", "9"),
            Topic::try_new(REGISTRATION_KEY).unwrap(),
            Topic::try_new(DEREGISTRATION_KEY).unwrap(),
            registration_response("7"),
            deregistration_response("7"),
            heartbeat_alert("robot7"),
            ruleset_pub_key("cell-7", "acme"),
            stop_cmd("fleet"),
            lidar_scan("fleet"),
            zone_entered("cell-3"),
            zone_cleared("cell-3"),
        ];
        for topic in topics {
            assert!(
                Topic::try_new(topic.as_str()).is_ok(),
                "builder produced invalid topic: {topic}"
            );
        }
    }

    #[test]
    fn builders_substitute_identifiers() {
        assert_eq!(robot_local("7", "bumper").as_str(), "robot/7/local/bumper");
        assert_eq!(rules_key("7").as_str(), "robot/7/local/rules");
        assert_eq!(liveliness_key("7").as_str(), "robot/7/client/liveliness");
        assert_eq!(signal_presence_key("7").as_str(), "robot/7/signal/presence");
        assert_eq!(
            signal_offer_key("7", "9").as_str(),
            "robot/7/signal/9/offer"
        );
        assert_eq!(signal_offer_pattern("9").as_str(), "robot/*/signal/9/offer");
        assert_eq!(
            registration_response("7").as_str(),
            "fleet/registration/response/7"
        );
        assert_eq!(
            heartbeat_alert("robot7").as_str(),
            "fleet/alerts/heartbeat/robot7"
        );
        assert_eq!(
            ruleset_pub_key("cell-7", "acme").as_str(),
            "fleet/cell-7/ruleset/acme"
        );
        assert_eq!(stop_cmd("fleet").as_str(), "stop/fleet/cmd");
        assert_eq!(zone_entered("cell-3").as_str(), "zone/cell-3/entered");
        assert_eq!(zone_cleared("cell-3").as_str(), "zone/cell-3/cleared");
    }

    #[test]
    fn invalid_empty() {
        let err = Topic::try_new("").unwrap_err();
        assert_eq!(err.kind, TopicErrorKind::Empty);
    }

    #[test]
    fn invalid_random() {
        let err = Topic::try_new("random/topic").unwrap_err();
        assert_eq!(err.kind, TopicErrorKind::UnknownPattern);
    }

    #[test]
    fn invalid_wrong_depth() {
        assert!(Topic::try_new("robot/7/local").is_err());
        assert!(Topic::try_new("robot/7").is_err());
        assert!(Topic::try_new("fleet/alerts/heartbeat").is_err());
    }

    #[test]
    fn invalid_underscoped() {
        assert!(Topic::try_new("custom/system/event").is_err());
        assert!(Topic::try_new("_internal/topic").is_err());
    }

    #[test]
    fn invalid_local_with_extra() {
        assert!(Topic::try_new("robot/7/local/bumper/x/y").is_err());
    }

    #[test]
    fn acl_expr_valid() {
        let a = robot_namespace("7");
        assert_eq!(a.as_str(), "/robot/7/**");
        assert!(AclExpr::try_new("/robot/7/**").is_ok());
        assert!(AclExpr::try_new("robot/7/**").is_err());
        assert!(AclExpr::try_new("/robot/7/*").is_err());
    }

    #[test]
    fn pattern_valid() {
        assert!(Pattern::try_new("robot/*/client/liveliness").is_ok());
        assert!(Pattern::try_new("fleet/*/ruleset/**").is_ok());
        assert!(Pattern::try_new("zone/*/entered").is_ok());
        assert!(Pattern::try_new("robot/*/signal/7/offer").is_ok());
        assert!(Pattern::try_new("").is_err());
    }

    #[test]
    fn topic_canonicalizes_hyphen() {
        // Single table: hyphen already canonicalized, so both forms match same arm
        let a = Topic::try_new("robot-7/local/bumper").unwrap();
        let b = Topic::try_new("robot/7/local/bumper").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "robot/7/local/bumper");
    }

    #[test]
    fn sensor_actuator_allowed_for_tests() {
        assert!(Topic::try_new("sensor/foo").is_ok());
        assert!(Topic::try_new("actuator/bar").is_ok());
        assert!(Topic::try_new("sensor/old").is_ok());
    }
}
