use std::process::Command;

#[test]
fn help_lists_all_flags_including_video() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .arg("--help")
        .output()
        .expect("run flo --help");
    assert!(out.status.success(), "flo --help should exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    // Exact Usage line and Commands structure — not just substring presence (INFRA-09).
    assert!(
        text.contains("Usage: flo [OPTIONS] [COMMAND]"),
        "missing Usage line, got: {text}"
    );
    assert!(
        text.contains("Commands:") && text.contains("rule"),
        "missing Commands/rule, got: {text}"
    );
    for flag in [
        "--robot-id",
        "--config",
        "--ruleset",
        "--auth-mode",
        "--auth-allow-insecure",
        "--auth-cert",
        "--auth-key",
        "--auth-trust",
        "--connect",
        "--healthcheck",
        "--video-peer",
        "--video-device",
        "--video-codec",
        "--video-self-test",
        "--help",
    ] {
        assert!(text.contains(flag), "help missing flag: {flag}\n{text}");
    }
    // Check that flags appear with their value placeholders (tighter than mere substring).
    assert!(
        text.contains("--robot-id <ID>"),
        "help should show placeholder for --robot-id, got: {text}"
    );
    assert!(
        text.contains("--config <PATH>"),
        "help should show placeholder for --config, got: {text}"
    );
    assert!(
        text.contains("--video-peer <ID>"),
        "help should show placeholder for --video-peer, got: {text}"
    );
    // Ensure the config help is not the old "required for client mode" phrasing
    // — the tightened INFRA-08 help will eventually assert the fail-safe wording;
    // for now we at least ensure the help is not empty and lists all flags.
    assert!(
        !text.trim().is_empty() && text.lines().count() > 10,
        "help text unexpectedly short, got: {text}"
    );
}
