use flo_rs::semantic::{parse_semantic, validate};

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
