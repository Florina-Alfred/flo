use flo_rs::rules::{Rules, When};
use flo_rs::semantic::{
    compile, compile_ruleset, parse_semantic, parse_semantic_ruleset, validate,
};

const DOC: &str = r#"
[site]
id = "cell-7"
frame = "cell-7/world"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "hrc-slow-near-human"
when.near_human = 1.2
actions = [ { slow_to = 0.1, qos = "best_effort" } ]
"#;

#[test]
fn parses_minimal_semantic_doc() {
    let doc = parse_semantic(DOC).expect("parse");
    assert_eq!(doc.site.id, "cell-7");
    assert_eq!(doc.zones.get("safety").unwrap().w, 2.0);
    assert_eq!(doc.rules.len(), 1);
    assert_eq!(doc.rules[0].when.near_human, Some(1.2));
}

#[test]
fn validates_good_doc_ok() {
    let doc = parse_semantic(DOC).unwrap();
    assert!(validate(&doc).is_ok());
}

#[test]
fn rejects_negative_distance() {
    let bad = r#"
[[rules]]
name = "x"
when.near_human = -1.0
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    let err = validate(&doc).unwrap_err();
    assert!(err.to_string().contains("distance"));
}

#[test]
fn rejects_unknown_action_verb() {
    // `explode` is not a known verb; an action with no known verb must fail validation.
    let bad = r#"
[[rules]]
name = "x"
when.in_zone = "safety"
actions = [ { explode = true } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    let err = validate(&doc).unwrap_err();
    assert!(err.to_string().contains("action"));
}

#[test]
fn compiles_near_human_to_trigger() {
    let doc = parse_semantic(DOC).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let r = &rules.rules[0];
    assert_eq!(r.name, "hrc-slow-near-human");
    // one trigger: topic fleet/cell-7/proximity/7/human; typed predicate pending #73 (currently None)
    let w: &When = &r.when;
    assert_eq!(w.all.len(), 1);
    assert_eq!(w.all[0].topic, "fleet/cell-7/proximity/7/human");
    // TODO(#73): once the typed predicate compiler lands, this becomes a typed
    // `Predicate` instead of `None`. Stopgap keeps the build green under the new
    // `Option<Predicate>` type.
    assert_eq!(w.all[0].pred, None);
    // one action: slow_to -> robot/7/local/drive, best_effort
    assert_eq!(r.actions.len(), 1);
    assert_eq!(r.actions[0].topic, "robot/7/local/drive");
    assert_eq!(r.actions[0].qos, flo_rs::rules::Qos::BestEffort);
}

#[test]
fn compile_rejects_unknown_zone() {
    let bad = r#"
[[rules]]
name = "x"
when.in_zone = "nope"
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    assert!(compile(&doc, "7").is_err());
}

#[test]
fn nested_when_any_produces_triggers() {
    let text = std::fs::read_to_string("examples/rules/hrc-cell.toml").expect("read hrc-cell.toml");
    let doc = parse_semantic(&text).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let protective = rules
        .rules
        .iter()
        .find(|r| r.name == "hrc-protective-stop-on-breach")
        .expect("protective-stop rule present");
    // The nested `when.any` must produce non-empty triggers — regression guard
    // against the silent no-op where unknown `all`/`any` keys were ignored.
    assert!(
        !protective.when.any.is_empty() || !protective.when.all.is_empty(),
        "nested when.any produced zero triggers (silent safety no-op)"
    );
    // The two branches: in_zone=="safety" and near_human<0.3.
    assert_eq!(protective.when.any.len(), 2);
    assert_eq!(protective.when.any[0].topic, "fleet/cell-7/7/state");
    assert_eq!(protective.when.any[0].pred, None);
    assert_eq!(
        protective.when.any[1].topic,
        "fleet/cell-7/proximity/7/human"
    );
    assert_eq!(protective.when.any[1].pred, None);
}

#[test]
fn nested_when_all_produces_triggers() {
    let text = std::fs::read_to_string("examples/rules/hrc-cell.toml").expect("read hrc-cell.toml");
    let doc = parse_semantic(&text).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let resume = rules
        .rules
        .iter()
        .find(|r| r.name == "hrc-resume-after-clear")
        .expect("resume rule present");
    // The nested `when.all` must produce non-empty triggers.
    assert!(
        !resume.when.all.is_empty() || !resume.when.any.is_empty(),
        "nested when.all produced zero triggers (silent safety no-op)"
    );
    assert_eq!(resume.when.all.len(), 2);
}

const RULESET_DOC: &str = r#"
ruleset_name = "acme-site-a"
version = 3
robot_owner = "robot/7"

[[rule]]
rule_name = "slow_near_human"
when.in_zone = "zone_1"
when.near_human = 1.2
when.human_presence = true
[[rule.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { speed_mps = 0.3 }
"#;

#[test]
fn parses_ruleset_envelope() {
    let doc = parse_semantic_ruleset(RULESET_DOC).expect("parse");
    assert_eq!(doc.ruleset_name, "acme-site-a");
    assert_eq!(doc.version, 3);
    assert_eq!(doc.robot_owner, "robot/7");
    assert_eq!(doc.rules.len(), 1);
}

#[test]
fn compiles_ruleset_to_envelope() {
    let doc = parse_semantic_ruleset(RULESET_DOC).unwrap();
    let rs: flo_rs::rules::Ruleset = compile_ruleset(&doc, "7").unwrap();
    assert_eq!(rs.ruleset_name, "acme-site-a");
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "slow_near_human");
}

#[test]
fn rejects_nonprimitive_payload() {
    let bad = r#"
ruleset_name = "x"
robot_owner = "robot/7"
[[rule]]
rule_name = "bad"
when.near_human = 1.0
[[rule.actions]]
topic = "robot/7/local/drive"
payload = { nested = { a = 1 } }
"#;
    let doc = parse_semantic_ruleset(bad).unwrap();
    assert!(compile_ruleset(&doc, "7").is_err());
}
