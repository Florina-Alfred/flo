use std::process::Command;

#[test]
fn rule_check_passes_valid_doc() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/rules/hrc-cell.toml");
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
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
    // write a temp bad doc
    let dir = std::env::temp_dir();
    let p = dir.join("flo-bad-rule.toml");
    std::fs::write(&p, "[[rules]]\nname=\"x\"\nwhen.near_human = -1.0\nactions = [ { slow_to = 0.1 } ]\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "check", p.to_str().unwrap()])
        .output()
        .expect("run flo rule check");
    assert!(!out.status.success(), "expected failure on bad doc");
}
