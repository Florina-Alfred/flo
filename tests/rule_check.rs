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
