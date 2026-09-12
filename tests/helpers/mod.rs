#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use flo_rs::config::ClientConfig;
use flo_rs::transport::Transport;

/// Return a free TCP port (bind to 0, read port, drop). Small sleep so OS
/// releases it before Zenoh binds.
pub fn get_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    std::thread::sleep(Duration::from_millis(10));
    p
}

fn router_config(port: u16) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    let _ = c.insert_json5("mode", "\"router\"");
    let _ = c.insert_json5("scouting/multicast/enabled", "false");
    let _ = c.insert_json5("scouting/gossip/enabled", "false");
    let _ = c.insert_json5("listen/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
    c
}

fn client_config(port: u16) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    let _ = c.insert_json5("mode", "\"client\"");
    let _ = c.insert_json5("scouting/multicast/enabled", "false");
    let _ = c.insert_json5("connect/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
    c
}

/// Open a Zenoh router on a free port. Returns the transport; its locators
/// can be queried via `transport.locators().await`.
pub async fn router_on_free_port() -> Transport {
    let port = get_free_port();
    let t = Transport::open_with(router_config(port))
        .await
        .expect("open router_on_free_port");
    tokio::time::sleep(Duration::from_millis(100)).await;
    t
}

/// Open a client transport connected to the given router's locator.
pub async fn client_for(router: &Transport) -> Transport {
    let locators = router.locators().await;
    assert!(
        !locators.is_empty(),
        "router should have at least one locator"
    );
    let endpoint = locators[0].clone();
    let mut c = zenoh::Config::default();
    let _ = c.insert_json5("mode", "\"client\"");
    let _ = c.insert_json5("scouting/multicast/enabled", "false");
    let _ = c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"));
    let client = Transport::open_with(c).await.expect("open client_for");
    tokio::time::sleep(Duration::from_millis(200)).await;
    client
}

/// Create a loopback pair: a router on a free port and a client connected to
/// it. Returns `(server, client)`. Both transports are ready to exchange
/// messages after a short propagation delay.
pub async fn loopback() -> (Transport, Transport) {
    let port = get_free_port();
    let server = Transport::open_with(router_config(port))
        .await
        .expect("open loopback server");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let client = Transport::open_with(client_config(port))
        .await
        .expect("open loopback client");
    tokio::time::sleep(Duration::from_millis(200)).await;
    (server, client)
}

/// Wait until `counter >= target` or `timeout` expires; panics on timeout.
/// Deadline-based polling (10ms interval) — hardened for CI load.
pub async fn wait_for_counter(counter: &AtomicU64, target: u64, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if counter.load(Ordering::SeqCst) >= target {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for eval_counter >= {target} (current {})",
                counter.load(Ordering::SeqCst)
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Poll `cond` until it returns true or `timeout` hits; returns true if
/// succeeded. Mirrors the old `poll_until` / deadline loops.
pub async fn poll_until<F, Fut>(mut cond: F, timeout: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if cond().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Sync helper for supervision's `try_wait` deadline loop (hardened, no flaky sleep).
pub fn wait_for_child_exit(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("wait child") {
            return Some(status);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Canonical minimal client config used across infra tests.
pub fn test_client_config() -> ClientConfig {
    ClientConfig::from_toml(
        r#"
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
"#,
    )
    .expect("test client config must parse")
}
