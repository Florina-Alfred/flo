use std::process::Command;

fn flo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_flo")
}

#[test]
fn rule_check_passes_valid_doc() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/rules/hrc-cell.toml");
    let out = Command::new(flo_bin())
        .args(["rule", "check", path])
        .output()
        .expect("run flo rule check");
    assert!(
        out.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rule_check_fails_invalid_doc() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-bad-rule.toml");
    std::fs::write(
        &p,
        "[[rules]]\nname=\"x\"\nwhen.near_human = -1.0\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check");
    assert!(!out.status.success(), "expected failure on bad doc");
}

#[test]
fn rule_check_json_output() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/rules/hrc-cell.toml");
    let out = Command::new(flo_bin())
        .args(["rule", "check", "--json", path])
        .output()
        .expect("run flo rule check --json");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(json["status"], "ok");
}

#[test]
fn rule_compile_outputs_json() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/rules/hrc-cell.toml");
    let out = Command::new(flo_bin())
        .args(["rule", "compile", path])
        .output()
        .expect("run flo rule compile");
    assert!(
        out.status.success(),
        "compile should succeed, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert!(json["rules"].is_array(), "output must have rules array");
    assert!(
        !json["rules"].as_array().unwrap().is_empty(),
        "must compile at least one rule"
    );
}

#[test]
fn rule_compile_fails_invalid_doc() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-compile-bad.toml");
    std::fs::write(
        &p,
        "[[rules]]\nname=\"x\"\nwhen.near_human = -1.0\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "compile", p.to_str().unwrap()])
        .output()
        .expect("run flo rule compile");
    assert!(!out.status.success(), "compile must fail on invalid doc");
}

#[test]
fn rule_check_accepts_json_input() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-check-json.json");
    std::fs::write(
        &p,
        r#"{"site":{"id":"cell-7"},"rules":[{"name":"r1","when":{"near_human":1.0},"actions":[{"slow_to":0.1}]}]}"#,
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check with JSON");
    assert!(
        out.status.success(),
        "JSON input should pass, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rule_compile_accepts_json_input() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-compile-json.json");
    std::fs::write(
        &p,
        r#"{"site":{"id":"cell-7"},"rules":[{"name":"r1","when":{"near_human":1.0},"actions":[{"slow_to":0.1}]}]}"#,
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "compile", p.to_str().unwrap()])
        .output()
        .expect("run flo rule compile with JSON");
    assert!(
        out.status.success(),
        "JSON input should compile, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(json["rules"][0]["name"], "r1");
}

#[test]
fn rule_compile_with_custom_robot_id() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-compile-robot-id.toml");
    std::fs::write(
        &p,
        "[site]\nid = \"cell-42\"\n[[rules]]\nname=\"r1\"\nwhen.near_human = 1.0\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "compile", p.to_str().unwrap(), "42"])
        .output()
        .expect("run flo rule compile with robot-id");
    assert!(
        out.status.success(),
        "compile with custom robot-id should succeed, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(
        json["rules"][0]["actions"][0]["topic"],
        "robot/42/local/drive"
    );
}

#[test]
fn rule_check_rejects_missing_file() {
    let out = Command::new(flo_bin())
        .args(["rule", "check", "/tmp/flo-nonexistent-file.toml"])
        .output()
        .expect("run flo rule check on missing file");
    assert!(!out.status.success(), "missing file should fail");
}

#[test]
fn rule_compile_rejects_missing_file() {
    let out = Command::new(flo_bin())
        .args(["rule", "compile", "/tmp/flo-nonexistent-file.toml"])
        .output()
        .expect("run flo rule compile on missing file");
    assert!(!out.status.success(), "missing file should fail");
}

#[test]
fn rule_rejects_unknown_subcommand() {
    let out = Command::new(flo_bin())
        .args(["rule", "bogus", "path"])
        .output()
        .expect("run flo rule bogus");
    assert!(!out.status.success(), "unknown subcommand should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // clap 4 prints "unrecognized subcommand", old manual code printed "unknown"
    assert!(
        stderr.contains("unknown")
            || stderr.contains("unrecognized")
            || stderr.contains("unrecognised"),
        "stderr should mention unknown/unrecognized subcommand, got: {stderr}"
    );
}

#[test]
fn rule_check_accepts_empty_file() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-empty.toml");
    std::fs::write(&p, "").unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check on empty file");
    assert!(
        out.status.success(),
        "empty file is valid (no rules), stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rule_check_rejects_bad_toml() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-bad-toml.toml");
    std::fs::write(&p, "[[broken = toml\n").unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check on bad toml");
    assert!(!out.status.success(), "bad toml should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("E001"),
        "stderr should contain error code E001"
    );
}

#[test]
fn rule_check_rejects_ruleset_envelope() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-envelope.toml");
    std::fs::write(
        &p,
        "ruleset_name = \"test\"\nversion = 1\nrobot_owner = \"robot/7\"\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check on envelope");
    // Envelope format parses as a SemanticDoc with no site, no rules — it's valid
    // TOML but semantically empty. The check should still succeed (empty ruleset
    // is valid) or at least not crash.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "empty ruleset should not crash: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn rule_check_json_output_on_failure() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-json-fail.toml");
    std::fs::write(
        &p,
        "[[rules]]\nname=\"x\"\nwhen.near_human = -1.0\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", "--json", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check --json on invalid doc");
    assert!(
        !out.status.success(),
        "invalid doc should fail even with --json"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON error output");
    assert_eq!(json["status"], "error");
}

#[test]
fn rule_check_rejects_typoed_when_key() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-typo-when.toml");
    std::fs::write(
        &p,
        "[[rules]]\nname=\"x\"\nwhen.in_zne = \"safety\"\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check on typo'd when key");
    assert!(
        !out.status.success(),
        "typo'd when key must fail, not silently produce an empty guard"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("in_zne"),
        "stderr should name the unknown key, got: {stderr}"
    );
}

#[test]
fn rule_check_rejects_empty_when() {
    let dir = std::env::temp_dir();
    let p = dir.join("flo-empty-when.toml");
    std::fs::write(
        &p,
        "[[rules]]\nname=\"x\"\nwhen = {}\nactions = [ { slow_to = 0.1 } ]\n",
    )
    .unwrap();
    let out = Command::new(flo_bin())
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check on empty when");
    assert!(
        !out.status.success(),
        "empty when must fail validation, not fire unconditionally"
    );
}

#[tokio::test]
async fn demo_rules_parse_with_field_operands() {
    // The built-in demo rules must parse into evaluable Field references, not
    // dead Str literals — the regression that made them impossible to fire.
    let rules = flo_rs::config::RuleStore::bootstrap_demo("7")
        .current()
        .await;
    let e_stop = rules
        .rules
        .iter()
        .find(|r| r.name == "e-stop-on-bumper")
        .expect("demo e-stop rule present");
    assert_eq!(e_stop.when.all.len(), 2);
    for t in &e_stop.when.all {
        let pred = t.pred.as_ref().expect("demo triggers carry a predicate");
        match pred {
            flo_rs::rules::Predicate::Comparison { lhs, .. } => {
                assert!(
                    matches!(lhs, flo_rs::rules::Operand::Field(_)),
                    "demo trigger must reference a payload Field, got {lhs:?}"
                );
            }
            other => panic!("unexpected demo predicate shape: {other:?}"),
        }
    }
}
