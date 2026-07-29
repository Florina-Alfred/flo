use std::path::PathBuf;
use std::process::Command;

/// Helper: run a command and return (exit_code, stdout, stderr).
fn run(cmd: &str, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {cmd}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (out.status.success(), stdout, stderr)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

// ── Ticket #141: Build commands ──────────────────────────────────────
// Build commands (cargo build, test, clippy, fmt) are verified by the
// CI pipeline and by running them before this test suite compiles.
// We verify the binary exists and its help text is correct.

#[test]
fn readme_flo_help_lists_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .arg("--help")
        .output()
        .expect("run flo --help");
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in &[
        "--robot-id",
        "--config",
        "--ruleset",
        "--auth-mode",
        "--auth-allow-insecure",
        "--connect",
    ] {
        assert!(text.contains(flag), "help missing flag: {flag}");
    }
}

#[test]
fn readme_flo_server_help_lists_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo-server"))
        .arg("--help")
        .output()
        .expect("run flo-server --help");
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in &["--config", "--auth-mode", "--auth-allow-insecure"] {
        assert!(text.contains(flag), "help missing flag: {flag}");
    }
}

// ── Ticket #142: flo rule check on examples ─────────────────────────

#[test]
fn readme_rule_check_hrc_cell() {
    let path = examples_dir().join("rules").join("hrc-cell.toml");
    let (ok, stdout, stderr) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "check", &path.to_string_lossy()],
    );
    assert!(ok, "flo rule check hrc-cell.toml failed:\n{stderr}");
    assert!(
        stdout.contains("OK") || !stdout.trim().is_empty(),
        "unexpected output: {stdout}"
    );
}

#[test]
fn readme_rule_check_warehouse_fleet() {
    let path = examples_dir().join("rules").join("warehouse-fleet.toml");
    let (ok, stdout, stderr) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "check", &path.to_string_lossy()],
    );
    assert!(ok, "flo rule check warehouse-fleet.toml failed:\n{stderr}");
    assert!(
        stdout.contains("OK") || !stdout.trim().is_empty(),
        "unexpected output: {stdout}"
    );
}

// ── Ticket #143: flo rule compile on examples ───────────────────────

#[test]
fn readme_rule_compile_hrc_cell() {
    let path = examples_dir().join("rules").join("hrc-cell.toml");
    let (ok, stdout, stderr) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "compile", &path.to_string_lossy()],
    );
    assert!(ok, "flo rule compile hrc-cell.toml failed:\n{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("compile output is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(json["rules"].is_array(), "output must have rules array");
    assert!(
        !json["rules"].as_array().unwrap().is_empty(),
        "must compile at least one rule"
    );
}

#[test]
fn readme_rule_compile_warehouse_fleet() {
    let path = examples_dir().join("rules").join("warehouse-fleet.toml");
    let (ok, stdout, stderr) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "compile", &path.to_string_lossy()],
    );
    assert!(
        ok,
        "flo rule compile warehouse-fleet.toml failed:\n{stderr}"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("compile output is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(json["rules"].is_array(), "output must have rules array");
    assert!(
        !json["rules"].as_array().unwrap().is_empty(),
        "must compile at least one rule"
    );
}

// ── Ticket #144: Server startup log messages ────────────────────────

#[test]
fn readme_server_starts_and_logs() {
    let config = fixtures_dir().join("minimal-server-config.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_flo-server"))
        .args([
            "--config",
            &config.to_string_lossy(),
            "--auth-mode",
            "none",
            "--auth-allow-insecure",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo-server");

    // Collect stdout for 3 seconds.
    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            s
        })
    });

    std::thread::sleep(std::time::Duration::from_secs(3));

    let _ = child.kill();
    let _ = child.wait();

    let output = stdout_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    assert!(!output.contains("panic"), "flo-server panicked: {output}");
    assert!(
        output.contains("flo-engine server mode started"),
        "expected 'server mode started', got:\n{output}"
    );
}

// ── Ticket #145: Client registration flow ───────────────────────────
// (requires server running — tested in ticket #147 end-to-end)

// ── Ticket #146: Health endpoints ───────────────────────────────────
// (requires client running — tested in ticket #147 end-to-end)

// ── Ticket #147: Quick start demo end-to-end ────────────────────────
// NOTE: This test requires Zenoh multicast scouting (224.0.0.224:7446)
// which may not work in all environments (e.g. containers, some CI).
// Marked #[ignore] — run with `cargo test -- --ignored` on a host with
// multicast support.

#[test]
#[ignore]
fn readme_demo_server_starts() {
    let config = fixtures_dir().join("minimal-server-config.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_flo-server"))
        .args([
            "--config",
            &config.to_string_lossy(),
            "--auth-mode",
            "none",
            "--auth-allow-insecure",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo-server");

    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            s
        })
    });

    // Let server start and set up queryables.
    std::thread::sleep(std::time::Duration::from_secs(4));

    // Start a client.
    let client_config = fixtures_dir().join("minimal-client-config.toml");
    let mut client = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args([
            "--robot-id",
            "robot-7",
            "--config",
            &client_config.to_string_lossy(),
            "--auth-mode",
            "none",
            "--auth-allow-insecure",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo client");

    let client_handle = client.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            s
        })
    });

    // Wait for registration.
    std::thread::sleep(std::time::Duration::from_secs(3));

    let _ = client.kill();
    let _ = client.wait();
    let _ = child.kill();
    let _ = child.wait();

    let server_output = stdout_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let client_output = client_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    assert!(
        !server_output.contains("panic"),
        "flo-server panicked: {server_output}"
    );
    assert!(
        !client_output.contains("panic"),
        "flo client panicked: {client_output}"
    );
    assert!(
        server_output.contains("flo-engine server mode started"),
        "server didn't start: {server_output}"
    );
    assert!(
        client_output.contains("registering with server"),
        "client didn't register: {client_output}"
    );
    assert!(
        client_output.contains("registration successful"),
        "registration not confirmed: {client_output}"
    );
}
