//! Topic naming convention — the single owner of the flo topic contract.
//!
//! Every topic the system publishes or subscribes to is named here: fixed
//! topics are `const`s, parameterized topics are builder functions, and the
//! subscription wildcards are pattern constants. No other module constructs
//! topic strings — callers build them through `topic::`, and this module's
//! tests verify every builder's output against [`check_topic_pattern`] so the
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

// --- Robot-local topics -----------------------------------------------------

/// A robot-local resource topic: `robot/{id}/local/{resource}`.
pub fn robot_local(robot_id: &str, resource: &str) -> String {
    format!("robot/{robot_id}/local/{resource}")
}

/// Robot ruleset hot-reload key: `robot/{id}/local/rules`.
pub fn rules_key(robot_id: &str) -> String {
    format!("robot/{robot_id}/local/rules")
}

// --- Liveliness -------------------------------------------------------------

/// Per-robot liveliness token topic: `robot/{id}/client/liveliness`.
pub fn liveliness_key(robot_id: &str) -> String {
    format!("robot/{robot_id}/client/liveliness")
}

/// Liveliness subscription pattern (all clients' tokens).
pub const LIVELINESS_PATTERN: &str = "robot/*/client/liveliness";

// --- Signaling (class-3 video) ----------------------------------------------

/// Presence advertisement topic: `robot/{id}/signal/presence`.
pub fn signal_presence_key(robot_id: &str) -> String {
    format!("robot/{robot_id}/signal/presence")
}

/// Presence subscription pattern (any robot's advertisement).
pub const SIGNAL_PRESENCE_PATTERN: &str = "robot/*/signal/presence";

/// Offer topic addressed to `peer_id`: `robot/{self}/signal/{peer}/offer`.
pub fn signal_offer_key(self_id: &str, peer_id: &str) -> String {
    format!("robot/{self_id}/signal/{peer_id}/offer")
}

/// Answer topic addressed to `peer_id`: `robot/{self}/signal/{peer}/answer`.
pub fn signal_answer_key(self_id: &str, peer_id: &str) -> String {
    format!("robot/{self_id}/signal/{peer_id}/answer")
}

/// Trickled-ICE topic addressed to `peer_id`: `robot/{self}/signal/{peer}/ice`.
pub fn signal_ice_key(self_id: &str, peer_id: &str) -> String {
    format!("robot/{self_id}/signal/{peer_id}/ice")
}

/// Offer subscription pattern: offers from any peer addressed to `self_id`.
pub fn signal_offer_pattern(self_id: &str) -> String {
    format!("robot/*/signal/{self_id}/offer")
}

/// Answer subscription pattern: answers from any peer addressed to `self_id`.
pub fn signal_answer_pattern(self_id: &str) -> String {
    format!("robot/*/signal/{self_id}/answer")
}

/// ICE subscription pattern: candidates from any peer addressed to `self_id`.
pub fn signal_ice_pattern(self_id: &str) -> String {
    format!("robot/*/signal/{self_id}/ice")
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
pub fn registration_response(robot_id: &str) -> String {
    format!("fleet/registration/response/{robot_id}")
}

/// Deregistration response topic for `robot_id`:
/// `fleet/deregistration/response/{robot_id}`.
pub fn deregistration_response(robot_id: &str) -> String {
    format!("fleet/deregistration/response/{robot_id}")
}

/// Heartbeat-alert topic for `robot_id`: `fleet/alerts/heartbeat/{robot_id}`.
pub fn heartbeat_alert(robot_id: &str) -> String {
    format!("fleet/alerts/heartbeat/{robot_id}")
}

// --- Fleet rulesets ---------------------------------------------------------

/// Fleet-scoped ruleset publish key: `fleet/{site}/ruleset/{name}`.
pub fn ruleset_pub_key(site: &str, name: &str) -> String {
    format!("fleet/{site}/ruleset/{name}")
}

/// Ruleset subscription pattern (any site, any name).
pub const RULESET_PUB_PATTERN: &str = "fleet/*/ruleset/**";

// --- Safety / sensors -------------------------------------------------------

/// Safety stop command topic: `stop/{scope}/cmd` (QoS class 1).
pub fn stop_cmd(scope: &str) -> String {
    format!("stop/{scope}/cmd")
}

/// LiDAR scan topic: `lidar/{scope}/scan` (QoS class 2).
pub fn lidar_scan(scope: &str) -> String {
    format!("lidar/{scope}/scan")
}

// --- Zone events ------------------------------------------------------------

/// Zone-entered event topic: `zone/{zone_id}/entered`.
pub fn zone_entered(zone_id: &str) -> String {
    format!("zone/{zone_id}/entered")
}

/// Zone-cleared event topic: `zone/{zone_id}/cleared`.
pub fn zone_cleared(zone_id: &str) -> String {
    format!("zone/{zone_id}/cleared")
}

/// Zone-entered subscription pattern (any zone).
pub const ZONE_ENTERED_PATTERN: &str = "zone/*/entered";

/// Zone-cleared subscription pattern (any zone).
pub const ZONE_CLEARED_PATTERN: &str = "zone/*/cleared";

// --- ACL namespaces ---------------------------------------------------------

/// Robot-scoped Zenoh ACL key-expression prefix: `/robot/{id}/**`. This is an
/// access-control key expression (leading slash, wildcard suffix), not a
/// concrete topic, so it is not covered by [`check_topic_pattern`].
pub fn robot_namespace(robot_id: &str) -> String {
    format!("/robot/{robot_id}/**")
}

/// Validate that a topic string follows the flo naming convention.
///
/// Returns `Ok(())` if the topic matches at least one known pattern, or
/// `Err` describing why it doesn't.
pub fn check_topic_pattern(topic: &str) -> Result<(), TopicError> {
    if topic.is_empty() {
        return Err(TopicError {
            topic: topic.to_string(),
            kind: TopicErrorKind::Empty,
        });
    }

    let parts: Vec<&str> = topic.split('/').collect();

    if matches_one_of(&parts) {
        Ok(())
    } else {
        Err(TopicError {
            topic: topic.to_string(),
            kind: TopicErrorKind::UnknownPattern,
        })
    }
}

fn is_robot_ns(s: &str) -> bool {
    s == "robot" || s.starts_with("robot-")
}

/// Check if the path segments match any known pattern.
fn matches_one_of(parts: &[&str]) -> bool {
    match *parts {
        // --- Robot-local slash form: robot/{id}/local/{sensor} (4 segments) ---
        [r, _, "local", "rules"] if is_robot_ns(r) => true,
        [r, _, "local", "cam0" | "cam1" | "cam2"] if is_robot_ns(r) => true,
        [r, _, "local", _] if is_robot_ns(r) => true,
        [r, _, "location", _] if is_robot_ns(r) => true,
        [r, _, "signal", "presence"] if is_robot_ns(r) => true,
        [r, _, "signal", _, _] if is_robot_ns(r) => true,
        [r, _, "zone"] if is_robot_ns(r) => true,
        [r, _, "site"] if is_robot_ns(r) => true,
        [r, _, "client", "liveliness"] if is_robot_ns(r) => true,
        // --- Robot-local hyphen form: robot-{id}/local/{sensor} (3 segments) ---
        [r, "local", "rules" | "cam0" | "cam1" | "cam2"] if is_robot_ns(r) => true,
        [r, "local", _] if is_robot_ns(r) => true,
        [r, "location", _] if is_robot_ns(r) => true,
        [r, "signal", "presence"] if is_robot_ns(r) => true,
        [r, "signal", _, _] if is_robot_ns(r) => true,
        [r, "zone"] if is_robot_ns(r) => true,
        [r, "site"] if is_robot_ns(r) => true,
        [r, "client", "liveliness"] if is_robot_ns(r) => true,
        // --- Fleet patterns ---
        ["fleet", "registration"] => true,
        ["fleet", "deregistration"] => true,
        ["fleet", "registration", "response", _] => true,
        ["fleet", "deregistration", "response", _] => true,
        ["fleet", "alerts", "heartbeat", _] => true,
        ["fleet", _, "ruleset", _] => true,
        // --- Safety / sensor ---
        ["stop", _, "cmd"] => true,
        ["lidar", _, "scan"] => true,
        // --- Zone events (entered/cleared grammar) ---
        ["zone", _, "entered"] | ["zone", _, "cleared"] => true,
        ["zone", _, _, _, "entered" | "cleared"] => true,
        // --- Catch-all for robot-local with too many segments ---
        [r, _, "local", _, _, ..] if is_robot_ns(r) => false,
        [r, "local", _, _, ..] if is_robot_ns(r) => false,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_robot_local() {
        assert!(check_topic_pattern("robot/7/local/bumper").is_ok());
        assert!(check_topic_pattern("robot/my-robot/local/drive").is_ok());
        assert!(check_topic_pattern("robot/7/local/human_present").is_ok());
    }

    #[test]
    fn rejects_too_many_local_segments() {
        assert!(check_topic_pattern("robot/7/local/bumper/extra").is_err());
    }

    #[test]
    fn valid_robot_location() {
        assert!(check_topic_pattern("robot/7/location/x").is_ok());
        assert!(check_topic_pattern("robot/7/location/y").is_ok());
        assert!(check_topic_pattern("robot/7/location/z").is_ok());
    }

    #[test]
    fn valid_robot_zone_site() {
        assert!(check_topic_pattern("robot/7/zone").is_ok());
        assert!(check_topic_pattern("robot/7/site").is_ok());
    }

    #[test]
    fn valid_fleet_topics() {
        assert!(check_topic_pattern("fleet/registration").is_ok());
        assert!(check_topic_pattern("fleet/deregistration").is_ok());
        assert!(check_topic_pattern("fleet/alerts/heartbeat/robot7").is_ok());
    }

    #[test]
    fn valid_fleet_response_topics() {
        assert!(check_topic_pattern("fleet/registration/response/robot-7").is_ok());
        assert!(check_topic_pattern("fleet/deregistration/response/robot-7").is_ok());
    }

    #[test]
    fn valid_fleet_ruleset() {
        assert!(check_topic_pattern("fleet/cell-7/ruleset/acme").is_ok());
    }

    #[test]
    fn valid_stop_and_lidar() {
        assert!(check_topic_pattern("stop/fleet/cmd").is_ok());
        assert!(check_topic_pattern("lidar/fleet/scan").is_ok());
    }

    #[test]
    fn valid_zone_events() {
        assert!(check_topic_pattern("zone/cell-3/entered").is_ok());
        assert!(check_topic_pattern("zone/cell-3/cleared").is_ok());
        assert!(check_topic_pattern("zone/site/cell/7/entered").is_ok());
        assert!(check_topic_pattern("zone/site/cell/7/cleared").is_ok());
    }

    #[test]
    fn rejects_non_canonical_zone_events() {
        // The `enter`/`exit` forms and the 4-segment `zone/{zone}/{id}/{event}`
        // spelling disagree with the entered/cleared grammar and are rejected.
        assert!(check_topic_pattern("zone/site/cell/7/enter").is_err());
        assert!(check_topic_pattern("zone/site/cell/7/exit").is_err());
        assert!(check_topic_pattern("zone/cell-3/7/enter").is_err());
        assert!(check_topic_pattern("zone/cell-3/7/exit").is_err());
    }

    #[test]
    fn valid_signal() {
        assert!(check_topic_pattern("robot/self/signal/presence").is_ok());
        assert!(check_topic_pattern("robot/self/signal/peer/offer").is_ok());
        assert!(check_topic_pattern("robot/self/signal/peer/answer").is_ok());
        assert!(check_topic_pattern("robot/self/signal/peer/ice").is_ok());
    }

    #[test]
    fn valid_hyphenated_robot_id() {
        assert!(check_topic_pattern("robot-7/local/bumper").is_ok());
        assert!(check_topic_pattern("robot-7/zone").is_ok());
    }

    #[test]
    fn all_builders_match_validator() {
        // Every concrete topic the system can emit must satisfy the validator,
        // so the builders and `check_topic_pattern` cannot disagree.
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
            REGISTRATION_KEY.to_string(),
            DEREGISTRATION_KEY.to_string(),
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
                check_topic_pattern(&topic).is_ok(),
                "builder produced invalid topic: {topic}"
            );
        }
    }

    #[test]
    fn builders_substitute_identifiers() {
        assert_eq!(robot_local("7", "bumper"), "robot/7/local/bumper");
        assert_eq!(rules_key("7"), "robot/7/local/rules");
        assert_eq!(liveliness_key("7"), "robot/7/client/liveliness");
        assert_eq!(signal_presence_key("7"), "robot/7/signal/presence");
        assert_eq!(signal_offer_key("7", "9"), "robot/7/signal/9/offer");
        assert_eq!(signal_offer_pattern("9"), "robot/*/signal/9/offer");
        assert_eq!(registration_response("7"), "fleet/registration/response/7");
        assert_eq!(heartbeat_alert("robot7"), "fleet/alerts/heartbeat/robot7");
        assert_eq!(
            ruleset_pub_key("cell-7", "acme"),
            "fleet/cell-7/ruleset/acme"
        );
        assert_eq!(stop_cmd("fleet"), "stop/fleet/cmd");
        assert_eq!(zone_entered("cell-3"), "zone/cell-3/entered");
        assert_eq!(zone_cleared("cell-3"), "zone/cell-3/cleared");
    }

    #[test]
    fn invalid_empty() {
        let err = check_topic_pattern("").unwrap_err();
        assert_eq!(err.kind, TopicErrorKind::Empty);
    }

    #[test]
    fn invalid_random() {
        let err = check_topic_pattern("random/topic").unwrap_err();
        assert_eq!(err.kind, TopicErrorKind::UnknownPattern);
    }

    #[test]
    fn invalid_wrong_depth() {
        assert!(check_topic_pattern("robot/7/local").is_err());
        assert!(check_topic_pattern("robot/7").is_err());
        assert!(check_topic_pattern("fleet/alerts/heartbeat").is_err());
    }

    #[test]
    fn invalid_underscoped() {
        assert!(check_topic_pattern("custom/system/event").is_err());
        assert!(check_topic_pattern("_internal/topic").is_err());
    }

    #[test]
    fn invalid_local_with_extra() {
        assert!(check_topic_pattern("robot/7/local/bumper/x/y").is_err());
    }
}
