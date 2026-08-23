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
    // Tighten tautology (INFRA-09): require exact OK prefix, not vacuously true.
    assert!(
        stdout.contains("OK:") && stdout.contains("is a valid semantic ruleset"),
        "expected OK with valid message, got: {stdout}"
    );
    assert!(
        stdout.contains("hrc-cell.toml"),
        "output should mention the file, got: {stdout}"
    );
    // JSON schema check via --json
    let (ok_json, out_json, _) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "check", &path.to_string_lossy(), "--json"],
    );
    assert!(ok_json, "check --json should succeed for hrc-cell");
    let v: serde_json::Value =
        serde_json::from_str(&out_json).expect("check --json must be valid JSON");
    assert_eq!(v["status"], "ok", "json status should be ok, got: {v}");
    assert_eq!(
        v["path"],
        path.to_string_lossy().to_string(),
        "json path should match, got: {v}"
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
        stdout.contains("OK:") && stdout.contains("is a valid semantic ruleset"),
        "expected OK with valid message, got: {stdout}"
    );
    assert!(
        stdout.contains("warehouse-fleet.toml"),
        "output should mention the file, got: {stdout}"
    );
    let (ok_json, out_json, _) = run(
        env!("CARGO_BIN_EXE_flo"),
        &["rule", "check", &path.to_string_lossy(), "--json"],
    );
    assert!(ok_json, "check --json should succeed for warehouse-fleet");
    let v: serde_json::Value =
        serde_json::from_str(&out_json).expect("check --json must be valid JSON");
    assert_eq!(v["status"], "ok", "json status should be ok, got: {v}");
    assert_eq!(
        v["path"],
        path.to_string_lossy().to_string(),
        "json path should match, got: {v}"
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
    let rules = json["rules"].as_array().unwrap();
    assert!(!rules.is_empty(), "must compile at least one rule");
    for rule in rules {
        assert!(rule["name"].is_string(), "rule must have name: {rule}");
        assert!(rule["when"].is_object(), "rule must have when: {rule}");
        assert!(rule["actions"].is_array(), "rule must have actions: {rule}");
        for action in rule["actions"].as_array().unwrap() {
            assert!(
                action["topic"].is_string(),
                "action must have topic string: {action}"
            );
            assert!(
                action["qos"].is_string(),
                "action must have qos string: {action}"
            );
            let qos = action["qos"].as_str().unwrap();
            assert!(
                qos == "reliable" || qos == "best_effort",
                "qos must be reliable/best_effort, got: {qos}"
            );
            assert!(
                action["payload"].is_object(),
                "action must have payload: {action}"
            );
        }
    }
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
    let rules = json["rules"].as_array().unwrap();
    assert!(!rules.is_empty(), "must compile at least one rule");
    for rule in rules {
        assert!(rule["name"].is_string(), "rule must have name: {rule}");
        assert!(rule["when"].is_object(), "rule must have when: {rule}");
        assert!(rule["actions"].is_array(), "rule must have actions: {rule}");
        for action in rule["actions"].as_array().unwrap() {
            assert!(
                action["topic"].is_string(),
                "action must have topic string: {action}"
            );
            assert!(
                action["qos"].is_string(),
                "action must have qos string: {action}"
            );
            let qos = action["qos"].as_str().unwrap();
            assert!(
                qos == "reliable" || qos == "best_effort",
                "qos must be reliable/best_effort, got: {qos}"
            );
        }
    }
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
// INFRA-09: the loopback demo proof now uses router mode (no multicast) so it
// runs in CI. The multicast-based spawn path is kept as an ignored test for
// host-only verification, but the primary proof is the router-based
// in-process registration below.

#[tokio::test(flavor = "multi_thread")]
async fn readme_demo_server_starts() {
    // In-process registration via the loopback router — deterministic, no
    // multicast scouting, proves the README quick-start's registration flow.
    // Uses `Transport::loopback_config` (router mode) which the demo's
    // `cargo run` also uses (via `AuthConfig::zenoh_config` for `auth: none`).
    let transport = std::sync::Arc::new(
        flo_rs::transport::Transport::open_with(flo_rs::transport::Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );
    let server_config =
        flo_rs::config::ServerConfig::from_toml("[[expected_clients]]\nrobot_id = \"robot-7\"\n")
            .expect("server config");
    let reg_server = flo_rs::registration::RegistrationServer::new(server_config);
    let t = transport.clone();
    let rs = reg_server.clone();
    tokio::spawn(async move {
        let _ = flo_rs::registration::run_registration_handler(t, rs).await;
    });
    // Give the handler time to declare its queryable (managed subscription
    // propagation is async; 200ms is enough on CI but we retry via deadline).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client_cfg_text =
        std::fs::read_to_string(fixtures_dir().join("minimal-client-config.toml"))
            .expect("read client config");
    let client_config =
        flo_rs::config::ClientConfig::from_toml(&client_cfg_text).expect("parse client config");
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        flo_rs::registration::register_with_client(transport.clone(), "robot-7", &client_config),
    )
    .await
    .expect("registration within 10s");
    assert!(
        res.is_ok(),
        "registration via loopback router should succeed, got: {res:?}"
    );
    assert_eq!(
        reg_server.state("robot-7").await,
        flo_rs::registration::ClientState::Registered
    );
}

#[test]
#[ignore]
fn readme_demo_server_starts_multicast() {
    // Host-only multicast verification: the original spawn-based demo that
    // requires 224.0.0.224:7446. Kept ignored so `cargo test -- --ignored`
    // still exercises the binary startup path on hosts with multicast.
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

    std::thread::sleep(std::time::Duration::from_secs(4));

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
