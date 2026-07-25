//! Topic naming convention validation.
//!
//! Defines the accepted topic patterns for the flo fleet orchestration system
//! and provides validation functions for rule triggers and actions.
//!
//! Convention (per existing codebase usage):
//!
//! | Category          | Pattern                                      |
//! |-------------------|----------------------------------------------|
//! | Robot-local       | `robot/{id}/local/{resource}`                |
//! | Robot location    | `robot/{id}/location/{axis}`                 |
//! | Robot zone        | `robot/{id}/zone`                            |
//! | Robot site        | `robot/{id}/site`                            |
//! | Robot liveliness  | `robot/{id}/client/liveliness`               |
//! | Robot rules       | `robot/{id}/local/rules`                     |
//! | Signaling         | `robot/{id}/signal/{peer}/{msgtype}`         |
//! | Camera            | `robot/{id}/local/cam{index}`                |
//! | Fleet registration| `fleet/registration`                         |
//! | Fleet dereg       | `fleet/deregistration`                       |
//! | Fleet alerts      | `fleet/alerts/heartbeat/{robot_id}`          |
//! | Fleet rulesets    | `fleet/{site}/ruleset/{name}`                |
//! | Safety stop       | `stop/{scope}/cmd`                           |
//! | LiDAR             | `lidar/{scope}/scan`                         |
//! | Zone events       | `zone/{zone_id}/entered`                     |
//! | Zone events       | `zone/{zone_id}/cleared`                     |
//! | Zone events       | `zone/{site}/{cell}/{robot_id}/{event}`      |

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
        [r, _, "signal", _, _] if is_robot_ns(r) => true,
        [r, _, "zone"] if is_robot_ns(r) => true,
        [r, _, "site"] if is_robot_ns(r) => true,
        [r, _, "client", "liveliness"] if is_robot_ns(r) => true,
        // --- Robot-local hyphen form: robot-{id}/local/{sensor} (3 segments) ---
        [r, "local", "rules" | "cam0" | "cam1" | "cam2"] if is_robot_ns(r) => true,
        [r, "local", _] if is_robot_ns(r) => true,
        [r, "location", _] if is_robot_ns(r) => true,
        [r, "signal", _, _] if is_robot_ns(r) => true,
        [r, "zone"] if is_robot_ns(r) => true,
        [r, "site"] if is_robot_ns(r) => true,
        [r, "client", "liveliness"] if is_robot_ns(r) => true,
        // --- Fleet patterns ---
        ["fleet", "registration"] => true,
        ["fleet", "deregistration"] => true,
        ["fleet", "alerts", "heartbeat", _] => true,
        ["fleet", _, "ruleset", _] => true,
        // --- Safety / sensor ---
        ["stop", _, "cmd"] => true,
        ["lidar", _, "scan"] => true,
        // --- Zone events ---
        ["zone", _, "entered"] | ["zone", _, "cleared"] => true,
        ["zone", _, _, _, _] => true,
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
                     zone/{{zone_id}}/{{event}}, or robot/{{id}}/signal/{{peer}}/{{msgtype}})",
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
        assert!(check_topic_pattern("zone/site/cell/7/enter").is_ok());
    }

    #[test]
    fn valid_signal() {
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
