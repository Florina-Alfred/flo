//! Ensures all default-feature examples compile and that their rule/topic
//! payloads are not vacuous stubs (INFRA-09: tighten tautology where build
//! success alone would pass with a no-op main). The `media`-feature example
//! requires system GStreamer, which is not present in CI; it is documented in
//! the README and verified manually where GStreamer is installed.

use std::process::Command;

#[test]
fn examples_compile_default() {
    let status = Command::new("cargo")
        .args(["build", "--examples"])
        .status()
        .expect("cargo build --examples");
    assert!(
        status.success(),
        "examples failed to build (default features)"
    );

    // Tighten beyond build success: the examples must actually reference
    // topics, QoS, and rule plumbing — a no-op main would otherwise pass.
    let custom =
        std::fs::read_to_string("examples/custom_rules.rs").expect("read custom_rules example");
    assert!(
        custom.contains("Transport") && custom.contains("RuleStore"),
        "custom_rules example should wire Transport + RuleStore, got: {custom}"
    );
    assert!(
        custom.contains("topic") || custom.contains("rules_key"),
        "custom_rules example should reference topics, got: {custom}"
    );

    let semantic =
        std::fs::read_to_string("examples/semantic_rules.rs").expect("read semantic_rules example");
    assert!(
        semantic.contains("compile") && semantic.contains("parse_semantic"),
        "semantic_rules example should compile semantic rules, got: {semantic}"
    );
    assert!(
        semantic.contains("topic") || semantic.contains("to_toml"),
        "semantic_rules example should handle topics/toml, got: {semantic}"
    );

    // Rule TOML fixtures that the examples load must contain real qos/topic
    // fields — not empty rules.
    for path in &[
        "examples/rules/hrc-cell.toml",
        "examples/rules/warehouse-fleet.toml",
    ] {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        assert!(
            text.contains("qos"),
            "rule file {path} should contain qos fields, got: {text}"
        );
        assert!(
            text.contains("[[rules]]") && text.contains("when."),
            "rule file {path} should contain rule definitions, got: {text}"
        );
        // hrc-cell uses semantic actions like slow_to/estop, warehouse uses near/in_zone
        assert!(
            text.contains("slow_to")
                || text.contains("estop")
                || text.contains("near")
                || text.contains("in_zone"),
            "rule file {path} should contain action primitives, got: {text}"
        );
    }

    // Compiled rule JSON must have topic/qos — verify via `flo rule compile`
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "compile", "examples/rules/hrc-cell.toml"])
        .output()
        .expect("run flo rule compile");
    assert!(
        out.status.success(),
        "compile hrc-cell failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("compile output is valid JSON");
    let rules = json["rules"].as_array().expect("rules array");
    assert!(!rules.is_empty(), "compiled rules must be non-empty");
    for rule in rules {
        assert!(rule["name"].is_string(), "rule must have name: {rule}");
        assert!(rule["actions"].is_array(), "rule must have actions: {rule}");
        for action in rule["actions"].as_array().unwrap() {
            assert!(
                action["topic"].is_string() && !action["topic"].as_str().unwrap().is_empty(),
                "action must have topic: {action}"
            );
            assert!(action["qos"].is_string(), "action must have qos: {action}");
            let qos = action["qos"].as_str().unwrap();
            assert!(
                qos == "reliable" || qos == "best_effort",
                "qos must be reliable/best_effort, got: {qos}"
            );
            assert!(
                action["payload"].is_object(),
                "action must have payload object: {action}"
            );
        }
        // when must have topics
        let when = &rule["when"];
        let all = when["all"].as_array().map(|v| v.len()).unwrap_or(0);
        let any = when["any"].as_array().map(|v| v.len()).unwrap_or(0);
        assert!(
            all > 0 || any > 0,
            "rule when must have at least one trigger, got: {rule}"
        );
    }
}
