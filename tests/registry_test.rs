use flo_rs::registry::{RegisterOutcome, Registry};
use flo_rs::rules::{Action, Qos, Rule, Ruleset, When};

fn sample_rs(name: &str, owner: &str, ver: u64) -> Ruleset {
    Ruleset {
        ruleset_name: name.to_string(),
        version: ver,
        robot_owner: owner.to_string(),
        rules: vec![Rule {
            name: "r".into(),
            when: When {
                all: vec![],
                any: vec![],
            },
            actions: vec![Action {
                topic: "robot/7/local/drive".into(),
                qos: Qos::Reliable,
                payload: serde_json::json!({ "speed_mps": 0.3 }),
            }],
        }],
    }
}

fn tmp_reg() -> Registry {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "flo-reg-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Registry::new(&dir.join("audit.db")).unwrap()
}

#[test]
fn new_name_inserts() {
    let reg = tmp_reg();
    let out = reg
        .publish(&sample_rs("acme", "robot/7", 1), "robot/7")
        .unwrap();
    assert!(matches!(out, RegisterOutcome::Inserted));
}

#[test]
fn same_owner_updates() {
    let reg = tmp_reg();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7")
        .unwrap();
    let out = reg
        .publish(&sample_rs("acme", "robot/7", 2), "robot/7")
        .unwrap();
    assert!(matches!(out, RegisterOutcome::Updated { version: 2, .. }));
}

#[test]
fn different_owner_rejects_with_conflict() {
    let reg = tmp_reg();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7")
        .unwrap();
    let out = reg
        .publish(&sample_rs("acme", "robot/9", 1), "robot/9")
        .unwrap();
    assert!(matches!(out, RegisterOutcome::RejectedConflict));
}

#[test]
fn sha_changes_bump_version_only_on_diff() {
    let reg = tmp_reg();
    reg.publish(&sample_rs("acme", "robot/7", 1), "robot/7")
        .unwrap();
    let out = reg
        .publish(&sample_rs("acme", "robot/7", 1), "robot/7")
        .unwrap();
    assert!(matches!(out, RegisterOutcome::Updated { version: 1, .. }));
}
