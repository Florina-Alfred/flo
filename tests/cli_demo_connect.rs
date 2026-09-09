use std::sync::Arc;

use flo_rs::auth::{AuthConfig, AuthMode};
use flo_rs::config::ClientConfig;
use flo_rs::registration::{RegistrationServer, run_registration_handler};
use flo_rs::transport::Transport;

/// Regression for #286: the README demo must work with explicit `--connect`
/// when multicast is blocked. This test exercises the exact CLI path:
/// `auth.zenoh_config` with `none` → `mode="router"` + `listen 127.0.0.1:0`,
/// server starts, client connects via `connect` to the server's locator,
/// and `register_with_client` succeeds within 10s. The old `Transport::loopback_config`
/// path is already covered by `safety_infra06::registration_envelope_loopback`;
/// this one covers the `auth.zenoh_config` + `--connect` path that the user hit.
#[tokio::test(flavor = "multi_thread")]
async fn cli_demo_registration_with_connect() {
    // Server: auth none → router + random listen, then get its locator
    let server_auth = AuthConfig {
        mode: AuthMode::None,
        allow_insecure: true,
        ..Default::default()
    };
    let server_config = server_auth.zenoh_config("7").expect("server auth config");
    let server_transport = Arc::new(
        Transport::open_with(server_config)
            .await
            .expect("open server transport"),
    );
    // Server must log its locators (new in #286 fix) — verify we can get them
    let locators = server_transport.locators().await;
    assert!(
        !locators.is_empty(),
        "server should have at least one locator, got {locators:?}"
    );
    assert!(
        locators.iter().any(|l| l.contains("tcp/127.0.0.1")),
        "server locators should be tcp/127.0.0.1:*, got {locators:?}"
    );
    let server_endpoint = locators[0].clone();
    // Extract just the locator string for connect (e.g. tcp/127.0.0.1:38247)
    // `locators()` already returns the full locator string.

    let reg_server = RegistrationServer::new(Default::default());
    let reg_transport = server_transport.clone();
    let reg_server_clone = reg_server.clone();
    tokio::spawn(async move {
        run_registration_handler(reg_transport, reg_server_clone)
            .await
            .expect("registration handler");
    });
    // Give the handler time to subscribe
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Client: auth none → router, but with explicit connect to server's locator
    let client_auth = AuthConfig {
        mode: AuthMode::None,
        allow_insecure: true,
        ..Default::default()
    };
    let client_config_zenoh = client_auth
        .zenoh_config("robot-7")
        .expect("client auth config");
    // Use Transport helper to merge --connect without touching insert_json5 directly
    let endpoints = vec![server_endpoint.clone()];
    let client_config_zenoh = Transport::with_endpoints(client_config_zenoh, &endpoints);
    let client_transport = Arc::new(
        Transport::open_with(client_config_zenoh)
            .await
            .expect("open client transport"),
    );

    let client_cfg = ClientConfig::from_toml(
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
    .expect("client config parse");

    // This is the exact call that timed out for the user without --connect.
    // With --connect it must succeed within 5s (the registration timeout).
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        flo_rs::registration::register_with_client(client_transport, "robot-7", &client_cfg),
    )
    .await
    .expect("registration should not hang beyond 10s");

    assert!(
        result.is_ok(),
        "registration with --connect should succeed, got {result:?} — locators were {locators:?}, server endpoint {server_endpoint}"
    );
}
