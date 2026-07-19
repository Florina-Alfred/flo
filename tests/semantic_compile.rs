use flo_rs::semantic::{compile, parse_semantic, validate};
use flo_rs::rules::{Rules, When};

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
    // one trigger: topic fleet/cell-7/proximity/7/human, pred separation_distance < 1.2
    let w: &When = &r.when;
    assert_eq!(w.all.len(), 1);
    assert_eq!(w.all[0].topic, "fleet/cell-7/proximity/7/human");
    assert_eq!(w.all[0].pred, Some("separation_distance < 1.2".to_string()));
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
