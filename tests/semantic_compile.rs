use flo_rs::rules::{EvalMode, Op, Operand, Predicate, PrimitiveRef, Rules, When};
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
    // `explode` is not a known verb; deny_unknown_fields rejects it at parse time.
    let bad = r#"
[[rules]]
name = "x"
when.in_zone = "safety"
actions = [ { explode = true } ]
"#;
    let err = parse_semantic(bad).unwrap_err();
    assert!(err.to_string().contains("explode"));
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
    assert_eq!(w.all[0].topic, "robot/7/local/human_present");
    assert_eq!(
        w.all[0].pred,
        Some(Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::HumanPresence),
            rhs: Operand::Float(1.2),
        })
    );
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
    // The two branches: in_zone=="safety" and near_human<0.3. After flattening
    // (#73 fix A) each nested `SemanticWhen` contributes its own trigger with its
    // own topic + predicate — NOT wrapped in `Predicate::Or`. The count stays 2.
    assert_eq!(protective.when.any.len(), 2);
    assert_eq!(protective.when.any[0].topic, "robot/7/local/zone");
    assert_eq!(
        protective.when.any[0].pred,
        Some(Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::Zone),
            rhs: Operand::Str("safety".into()),
        })
    );
    assert_eq!(protective.when.any[1].topic, "robot/7/local/human_present");
    assert_eq!(
        protective.when.any[1].pred,
        Some(Predicate::Comparison {
            op: Op::Lt,
            lhs: Operand::Prim(PrimitiveRef::HumanPresence),
            rhs: Operand::Float(0.3),
        })
    );
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
fn compiles_in_zone_to_typed_predicate() {
    let doc = parse_semantic_ruleset(
        r#"
ruleset_name = "x"
robot_owner = "robot/7"
[[rule]]
rule_name = "r"
when.in_zone = "zone_1"
[[rule.actions]]
topic = "robot/7/local/drive"
payload = { speed_mps = 0.3 }
"#,
    )
    .unwrap();
    let rs = compile_ruleset(&doc, "7").unwrap();
    let t = &rs.rules[0].when.all[0];
    assert_eq!(
        t.pred,
        Some(Predicate::Comparison {
            op: Op::Eq,
            lhs: Operand::Prim(PrimitiveRef::Zone),
            rhs: Operand::Str("zone_1".into()),
        })
    );
    // zone entry is an edge event
    assert_eq!(t.mode, EvalMode::Edge);
}

#[test]
fn action_targets_prd5_local_drive() {
    let doc = parse_semantic_ruleset(RULESET_DOC).unwrap();
    let rs = compile_ruleset(&doc, "7").unwrap();
    assert_eq!(rs.rules[0].actions[0].topic, "robot/7/local/drive");
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

#[test]
fn rejects_empty_when() {
    let bad = r#"
[site]
id = "cell-7"
[[rules]]
name = "empty-guard"
when = {}
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    let err = validate(&doc).unwrap_err();
    assert!(err.to_string().contains("empty"), "got: {err}");
    assert!(compile(&doc, "7").is_err());
}

#[test]
fn rejects_nested_empty_when() {
    let bad = r#"
[site]
id = "cell-7"
[[rules]]
name = "nested-empty"
when.all = [ {} ]
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(bad).unwrap();
    assert!(validate(&doc).is_err());
    assert!(compile(&doc, "7").is_err());
}

#[test]
fn rejects_unknown_when_field() {
    let bad = r#"
[[rules]]
name = "typo"
when.in_zone = "safety"
when.inn_zone = "safety"
actions = [ { slow_to = 0.1 } ]
"#;
    let err = parse_semantic(bad).unwrap_err();
    assert!(err.to_string().contains("inn_zone"), "got: {err}");
}

#[test]
fn nested_all_with_any_compiles() {
    // `all = [{ any = [...] }]` must not silently drop the nested OR group
    // (regression: nested `any` used to vanish, leaving an empty guard that
    // fired every tick).
    let text = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "x"
when.all = [
  { any = [ { in_zone = "safety" }, { near_human = 0.3 } ] },
]
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(text).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let w = &rules.rules[0].when;
    assert_eq!(w.all.len(), 0);
    assert_eq!(w.any.len(), 2);
}

#[test]
fn nested_any_with_any_compiles() {
    // `any = [{ any = [...] }]` flattens OR-of-OR into a single OR group.
    let text = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "x"
when.any = [
  { any = [ { in_zone = "safety" }, { near_human = 0.3 } ] },
]
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(text).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let w = &rules.rules[0].when;
    assert_eq!(w.all.len(), 0);
    assert_eq!(w.any.len(), 2);
}

#[test]
fn rejects_two_or_groups_anded() {
    // `all = [{ any = [...] }, { any = [...] }]` is (A||B) && (C||D), which the
    // two-level runtime `When` cannot express — fail closed rather than drop one.
    let text = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "x"
when.all = [
  { any = [ { in_zone = "safety" } ] },
  { any = [ { near_human = 0.3 } ] },
]
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(text).unwrap();
    let err = compile(&doc, "7").unwrap_err();
    assert!(err.to_string().contains("OR group"), "got: {err}");
}

#[test]
fn rejects_conjunction_inside_or_element() {
    // An `any` element that is an AND of two triggers cannot be a single OR term.
    let text = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "x"
when.any = [
  { in_zone = "safety", near_human = 0.3 },
]
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(text).unwrap();
    let err = compile(&doc, "7").unwrap_err();
    assert!(err.to_string().contains("OR"), "got: {err}");
}

#[test]
fn flat_when_still_compiles() {
    // A flat condition must remain untouched by the nesting rules.
    let text = r#"
[site]
id = "cell-7"
[zones]
safety = { shape = "rect", x = 0.0, y = 0.0, w = 2.0, h = 2.0 }
[[rules]]
name = "x"
when.in_zone = "safety"
when.near_human = 0.3
actions = [ { slow_to = 0.1 } ]
"#;
    let doc = parse_semantic(text).unwrap();
    let rules: Rules = compile(&doc, "7").unwrap();
    let w = &rules.rules[0].when;
    assert_eq!(w.all.len(), 2);
    assert!(w.any.is_empty());
}
