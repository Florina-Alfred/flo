use std::process::Command;

#[test]
fn help_lists_all_flags_including_video() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .arg("--help")
        .output()
        .expect("run flo --help");
    assert!(out.status.success(), "flo --help should exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    // Structure: usage line, commands, options
    assert!(
        text.contains("Usage: flo [OPTIONS] [COMMAND]"),
        "missing Usage line, got: {text}"
    );
    assert!(
        text.contains("Commands:") && text.contains("rule"),
        "missing Commands/rule, got: {text}"
    );
    // Required flags with exact help snippets
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
    // --config must be fail-safe wording, not "required for client mode"
    assert!(
        text.contains("Optional. Missing/unreadable"),
        "missing fail-safe --config help, got: {text}"
    );
    assert!(
        text.contains("fail-safe empty ruleset"),
        "missing fail-safe phrase, got: {text}"
    );
    assert!(
        text.contains("built-in demo rules"),
        "missing demo rules phrase, got: {text}"
    );
    assert!(
        !text.contains("required for client mode"),
        "old --config help still present, got: {text}"
    );
    // ed25519 must note not yet implemented — fails closed
    assert!(
        text.contains("ed25519") && text.contains("not yet implemented"),
        "ed25519 help must note not yet implemented, got: {text}"
    );
    assert!(
        text.contains("fails closed"),
        "ed25519 help must note fails closed, got: {text}"
    );
    // media hint on video
    assert!(
        text.contains("media") && text.contains("GStreamer"),
        "video help must mention media/GStreamer, got: {text}"
    );
}

#[test]
fn rule_help_lists_subcommands() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "--help"])
        .output()
        .expect("run flo rule --help");
    assert!(out.status.success(), "flo rule --help should exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Usage: flo rule <COMMAND>"),
        "rule --help should show Usage: flo rule <COMMAND>, got: {text}"
    );
    assert!(
        text.contains("check") && text.contains("compile"),
        "rule --help must list check and compile, got: {text}"
    );
    // Old broken help was `Usage: flo rule [ARGS]...` — must not appear
    assert!(
        !text.contains("[ARGS]"),
        "old trailing_var_arg help still present, got: {text}"
    );
    assert!(
        text.contains("Validate the ruleset at PATH") || text.contains("Validate the ruleset"),
        "check help missing, got: {text}"
    );
}

#[test]
fn rule_check_help_shows_path_and_json() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "check", "--help"])
        .output()
        .expect("run flo rule check --help");
    assert!(
        out.status.success(),
        "flo rule check --help should exit 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Usage: flo rule check"),
        "missing Usage: flo rule check, got: {text}"
    );
    assert!(
        text.contains("<PATH>") || text.contains("PATH"),
        "missing PATH arg, got: {text}"
    );
    assert!(text.contains("--json"), "missing --json flag, got: {text}");
}

#[test]
fn rule_compile_help_shows_path_and_robot_id() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["rule", "compile", "--help"])
        .output()
        .expect("run flo rule compile --help");
    assert!(
        out.status.success(),
        "flo rule compile --help should exit 0"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Usage: flo rule compile"),
        "missing Usage: flo rule compile, got: {text}"
    );
    assert!(text.contains("PATH"), "missing PATH, got: {text}");
    assert!(
        text.contains("ROBOT_ID") || text.contains("robot_id"),
        "missing ROBOT_ID, got: {text}"
    );
}

#[test]
fn server_help_hides_client_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo-server"))
        .arg("--help")
        .output()
        .expect("run flo-server --help");
    assert!(out.status.success(), "flo-server --help should exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Usage: flo-server"),
        "missing Usage: flo-server, got: {text}"
    );
    // Must NOT contain client-only flags as option definitions.
    // Note: --config help mentions "--ruleset" in its description, so we check
    // for the flag definition pattern, not any substring.
    assert!(
        !text
            .lines()
            .any(|l| l.trim_start().starts_with("--ruleset ")),
        "flo-server --help should hide --ruleset flag, got: {text}"
    );
    assert!(
        !text
            .lines()
            .any(|l| l.trim_start().starts_with("--ruleset<")),
        "flo-server --help should hide --ruleset flag, got: {text}"
    );
    for flag in [
        "--video-peer",
        "--video-device",
        "--video-codec",
        "--video-self-test",
    ] {
        assert!(
            !text.contains(flag),
            "flo-server --help should hide {flag}, but found it in: {text}"
        );
    }
    // Must still contain shared flags
    for flag in [
        "--robot-id",
        "--config",
        "--auth-mode",
        "--auth-cert",
        "--auth-key",
        "--auth-trust",
        "--connect",
        "--healthcheck",
    ] {
        assert!(
            text.contains(flag),
            "flo-server help missing shared flag {flag}, got: {text}"
        );
    }
    // --config fail-safe wording on server too
    assert!(
        text.contains("Optional. Missing/unreadable"),
        "server --config must have fail-safe help, got: {text}"
    );
    // ed25519 note on server too
    assert!(
        text.contains("not yet implemented") && text.contains("fails closed"),
        "server ed25519 help must note fails closed, got: {text}"
    );
}
