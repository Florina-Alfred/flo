mod helpers;

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use flo_rs::cli::Args;
use flo_rs::config::ActiveRules;
use flo_rs::engine;
use flo_rs::health::ReadyGate;
use flo_rs::runtime::{Runtime, start_common_subsystems};
use flo_rs::transport::Transport;
use helpers::wait_for_child_exit;

fn empty_store() -> ActiveRules {
    ActiveRules::bootstrap("rules = []\n").expect("empty ruleset always parses")
}

/// Killing one subsystem must be detected by the client's supervision, which
/// returns an error (the binary turns it into a non-zero exit).
#[tokio::test(flavor = "multi_thread")]
async fn dead_engine_is_detected_by_supervision() {
    // Use helpers::loopback to ensure router/client pair is correctly wired
    // (demonstrates shared harness usage; single-transport loopback still ok).
    let (server, _client) = helpers::loopback().await;
    let transport = Arc::new(server);
    let store = empty_store();
    let args = Args::parse_from(["flo", "--auth-mode", "none", "--auth-allow-insecure"]);
    let gate = ReadyGate::new();

    let handles = start_common_subsystems(&transport, &store, "robot-7", &args, gate).await;

    // Kill the rule engine subsystem; supervision must take the client down.
    handles.engine.abort();

    let err = Runtime::supervise(handles)
        .await
        .expect_err("supervision must fail when a subsystem dies");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("rule engine") || msg.contains("signaling") || msg.contains("subsystem"),
        "expected supervision to report a dead subsystem (engine/signaling), got: {err}"
    );
}

/// The engine consumes the ReadyGate token and flips it once its sensor
/// subscriptions are live, so `/readyz` can never flip before subscription.
#[tokio::test(flavor = "multi_thread")]
async fn engine_confirms_subscriptions_on_ready_gate() {
    let transport = Arc::new(
        Transport::open_router()
            .await
            .expect("open loopback transport"),
    );
    let store = empty_store();

    let gate = ReadyGate::new();
    let gate_clone = gate.clone();
    let t = transport.clone();
    let s = store.clone();
    let task = tokio::spawn(async move {
        engine::run_engine(t, s, gate_clone)
            .await
            .expect("engine run");
    });

    // Poll gate.is_ready_public until engine flips it, bounded by 5s deadline.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if gate.is_ready_public() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "engine must confirm subscriptions within 5s"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    task.abort();
}

/// End-to-end: make one subsystem die in the real binary (hold its health
/// port so the health server cannot bind) and assert the process exits
/// non-zero with a fatal log instead of lingering unmonitored.
#[test]
fn dead_health_subsystem_makes_client_exit_nonzero() {
    // Holding the listener makes the client's health-server bind fail on the
    // same addr:port, killing the health subsystem at startup.
    let holder = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe port");
    let addr = format!("127.0.0.1:{}", holder.local_addr().unwrap().port());

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_flo"))
        .args(["--auth-mode", "none", "--auth-allow-insecure"])
        .env("FLO_HEALTH_ADDR", &addr)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo client");

    // Use shared harness helper for deadline-based poll (was duplicated inline).
    let status = wait_for_child_exit(&mut child, Duration::from_secs(30))
        .expect("client stayed alive after its health subsystem died");

    let stdout = read_all(&mut child.stdout.take().unwrap());
    let stderr = read_all(&mut child.stderr.take().unwrap());

    assert!(
        !status.success(),
        "client must exit non-zero when a subsystem dies\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("fatal"),
        "supervision must log the death as fatal, stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("health") || stderr.contains("health"),
        "exit must be tied to the dead health subsystem\nstdout: {stdout}\nstderr: {stderr}"
    );
}

fn read_all(stream: &mut impl std::io::Read) -> String {
    let mut s = String::new();
    let _ = stream.read_to_string(&mut s);
    s
}
