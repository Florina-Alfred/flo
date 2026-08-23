use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use clap::Parser;

use flo_rs::cli::Args;
use flo_rs::common::start_common_subsystems;
use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::runtime::ClientRuntime;
use flo_rs::transport::Transport;

fn empty_store() -> RuleStore {
    RuleStore::bootstrap("rules = []\n").expect("empty ruleset always parses")
}

/// Killing one subsystem must be detected by the client's supervision, which
/// returns an error (the binary turns it into a non-zero exit).
#[tokio::test(flavor = "multi_thread")]
async fn dead_engine_is_detected_by_supervision() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );
    let store = empty_store();
    let args = Args::parse_from(["flo", "--auth-mode", "none", "--auth-allow-insecure"]);

    let handles = start_common_subsystems(&transport, &store, "robot-7", &args).await;

    // Kill the rule engine subsystem; supervision must take the client down.
    handles.engine.abort();

    let err = ClientRuntime::supervise(handles)
        .await
        .expect_err("supervision must fail when a subsystem dies");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("rule engine") || msg.contains("signaling") || msg.contains("subsystem"),
        "expected supervision to report a dead subsystem (engine/signaling), got: {err}"
    );
}

/// The engine reports through the ready-gate channel only once its sensor
/// subscriptions are live, so `/readyz` can never flip before subscription.
#[tokio::test(flavor = "multi_thread")]
async fn engine_confirms_subscriptions_on_ready_gate() {
    let transport = Arc::new(
        Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport"),
    );
    let store = empty_store();

    let (tx, rx) = tokio::sync::oneshot::channel();
    let counter = Arc::new(AtomicU64::new(0));
    let t = transport.clone();
    let s = store.clone();
    let c = counter.clone();
    let task = tokio::spawn(async move {
        engine::run_engine(t, s, c, Some(tx))
            .await
            .expect("engine run");
    });

    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("engine must confirm subscriptions within 5s")
        .expect("subscribed signal must fire");

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

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait child") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "client stayed alive after its health subsystem died"
        );
        std::thread::sleep(Duration::from_millis(50));
    };

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
