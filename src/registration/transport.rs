//! Registration transport — owns typed envelopes `RegistrationRequest::Register`
//! / `RegistrationResponse` (serde `tag="op"`) and the
//! `fleet/registration` + `fleet/registration/response/{robot_id}` +
//! `fleet/deregistration` topics. Interface: `register(robot_id, config) ->
//! Result<(), RegistrationError>` — blocking, no retry loop. Retry/backoff
//! (`3× + 1s*attempt`) is caller policy in `runtime.rs`, not here.
//!
//! `SampleKind::Put/Delete` and raw `put_bytes` stay behind this seam —
//! callers never see `zenoh::sample::Sample`.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use zenoh::sample::SampleKind;

use crate::config::ClientConfig;
use crate::registration::lease::{RegistrationError, RegistrationLease, RegistrationServer};
use crate::transport::{Envelope, Transport};

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Client → server registration envelope, discriminated by `op`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RegistrationRequest {
    Register {
        robot_id: String,
        config: Box<ClientConfig>,
    },
    Deregister {
        robot_id: String,
    },
}

/// Server → client response envelope carrying a discriminated status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationResponse {
    pub status: RegistrationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationStatus {
    Ack,
    RejectAlreadyRegistered,
    RejectPoisoned,
    RejectServerError(String),
    Ignore,
    MissingRobotId,
    Poisoned,
}

/// Serialize a typed response and publish it to `topic`.
async fn publish_response(
    transport: &Transport,
    topic: crate::topic::Topic,
    status: RegistrationStatus,
) -> Result<(), RegistrationError> {
    let payload = serde_json::to_value(&RegistrationResponse { status }).map_err(|e| {
        RegistrationError::ServerError(format!("failed to serialize response: {e}"))
    })?;
    transport
        .publish(topic, Envelope::Signal(payload))
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))
}

pub async fn run_registration_handler(
    transport: Arc<Transport>,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let reg = reg_server.clone();
    let transport_for_reg = transport.clone();
    let pattern = crate::topic::Pattern::try_new(crate::topic::REGISTRATION_KEY).unwrap();
    let sub = transport.subscribe(pattern).await?;
    let reg_task = {
        let reg = reg.clone();
        let transport = transport_for_reg.clone();
        tokio::spawn(async move {
            while let Ok(sample) = sub.recv_async().await {
                let bytes = sample.payload().to_bytes();
                let request: RegistrationRequest = match serde_json::from_slice(&bytes) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!("registration: bad request: {e}");
                        continue;
                    }
                };
                let RegistrationRequest::Register { robot_id, config } = request else {
                    warn!(
                        "registration: non-register request on {}",
                        crate::topic::REGISTRATION_KEY
                    );
                    continue;
                };
                let status = if robot_id.is_empty() {
                    RegistrationStatus::MissingRobotId
                } else {
                    match reg.register(&robot_id, *config).await {
                        Ok(()) => RegistrationStatus::Ack,
                        Err(RegistrationError::AlreadyRegistered) => {
                            RegistrationStatus::RejectAlreadyRegistered
                        }
                        Err(RegistrationError::Poisoned) => RegistrationStatus::RejectPoisoned,
                        Err(e) => RegistrationStatus::RejectServerError(format!("{e:?}")),
                    }
                };
                let response_key = crate::topic::registration_response(&robot_id);
                let _ = publish_response(&transport, response_key, status).await;
            }
        })
    };

    info!(
        "registration subscriber active on {}",
        crate::topic::REGISTRATION_KEY
    );

    let dereg_pattern = crate::topic::Pattern::try_new(crate::topic::DEREGISTRATION_KEY).unwrap();
    let dereg_sub = transport.subscribe(dereg_pattern).await?;
    let dereg_task = {
        let reg = reg_server.clone();
        let transport = transport.clone();
        tokio::spawn(async move {
            while let Ok(sample) = dereg_sub.recv_async().await {
                let bytes = sample.payload().to_bytes();
                let request: RegistrationRequest = match serde_json::from_slice(&bytes) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!("deregistration: bad request: {e}");
                        continue;
                    }
                };
                let RegistrationRequest::Deregister { robot_id } = request else {
                    warn!(
                        "deregistration: non-deregister request on {}",
                        crate::topic::DEREGISTRATION_KEY
                    );
                    continue;
                };
                let response_key = crate::topic::deregistration_response(&robot_id);
                let status = if robot_id.is_empty() {
                    RegistrationStatus::MissingRobotId
                } else {
                    match reg.deregister(&robot_id).await {
                        Ok(()) => RegistrationStatus::Ack,
                        Err(_) => RegistrationStatus::Ignore,
                    }
                };
                let _ = publish_response(&transport, response_key, status).await;
            }
        })
    };

    // Keep tasks alive until pending (never returns)
    let _ = tokio::join!(reg_task, dereg_task);
    std::future::pending::<()>().await;
    Ok(())
}

pub async fn run_heartbeat_monitor(
    transport: Arc<Transport>,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let transport_for_alert = transport.clone();
    let pattern = crate::topic::Pattern::try_new(crate::topic::LIVELINESS_PATTERN).unwrap();
    let sub = transport.subscribe(pattern).await?;

    // Keep a clone for the spawned task. `RegistrationLease` is `Clone` via
    // `Arc<RwLock<...>>`, so this shares the same state as the registration
    // handler — poison decisions are globally visible.
    let lease_for_task: RegistrationLease = reg_server;
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let key = sample.key_expr().to_string();
            let kind = sample.kind();
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() < 2 {
                continue;
            }
            let robot_id = parts[1].to_string();
            match kind {
                SampleKind::Put => {
                    info!(%robot_id, "heartbeat: client alive");
                }
                SampleKind::Delete => {
                    if lease_for_task.on_liveliness_delete(&robot_id).await {
                        let alert_topic = crate::topic::heartbeat_alert(&robot_id);
                        let _ = publish_response(
                            &transport_for_alert,
                            alert_topic,
                            RegistrationStatus::Poisoned,
                        )
                        .await;
                    }
                }
            }
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}

/// Blocking registration: single attempt, no retry loop. Caller (e.g.
/// `runtime.rs`) owns the `3× + 1s*attempt` retry/backoff policy.
pub async fn register_with_client(
    transport: Arc<Transport>,
    robot_id: &str,
    config: &ClientConfig,
) -> Result<(), RegistrationError> {
    if robot_id.is_empty() {
        return Err(RegistrationError::ServerError("MissingRobotId".to_string()));
    }
    let request = RegistrationRequest::Register {
        robot_id: robot_id.to_string(),
        config: Box::new(config.clone()),
    };
    let request_value = serde_json::to_value(&request)
        .map_err(|e| RegistrationError::ServerError(format!("failed to serialize request: {e}")))?;

    let response_topic = crate::topic::registration_response(robot_id);
    let response_pattern = crate::topic::Pattern::try_new(response_topic.as_str())
        .expect("response topic is valid pattern");

    // Subscribe to response topic before sending request.
    let response_sub = transport
        .subscribe(response_pattern)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    // Send registration request.
    let reg_topic = crate::topic::Topic::try_new(crate::topic::REGISTRATION_KEY)
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;
    transport
        .publish(reg_topic, Envelope::Signal(request_value))
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    // Wait for response with timeout.
    let response = tokio::time::timeout(REGISTRATION_TIMEOUT, async {
        response_sub
            .recv_async()
            .await
            .ok()
            .map(|sample| sample.payload().to_bytes().to_vec())
    })
    .await;

    let bytes = match response {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(_) => return Err(RegistrationError::Timeout),
    };

    let resp: RegistrationResponse = serde_json::from_slice(&bytes)
        .map_err(|e| RegistrationError::ServerError(format!("bad registration response: {e}")))?;

    match resp.status {
        RegistrationStatus::Ack => {
            info!(robot_id, "registration successful");
            Ok(())
        }
        RegistrationStatus::RejectAlreadyRegistered => Err(RegistrationError::AlreadyRegistered),
        RegistrationStatus::RejectPoisoned => Err(RegistrationError::Poisoned),
        RegistrationStatus::RejectServerError(msg) => Err(RegistrationError::ServerError(msg)),
        RegistrationStatus::MissingRobotId => {
            Err(RegistrationError::ServerError("MissingRobotId".to_string()))
        }
        status => Err(RegistrationError::ServerError(format!(
            "unexpected registration status: {status:?}"
        ))),
    }
}

pub async fn deregister_with_server(
    transport: Arc<Transport>,
    robot_id: &str,
) -> Result<(), RegistrationError> {
    let request = RegistrationRequest::Deregister {
        robot_id: robot_id.to_string(),
    };
    let request_value = serde_json::to_value(&request)
        .map_err(|e| RegistrationError::ServerError(format!("failed to serialize request: {e}")))?;

    let response_topic = crate::topic::deregistration_response(robot_id);
    let response_pattern =
        crate::topic::Pattern::try_new(response_topic.as_str()).expect("response topic valid");

    let response_sub = transport
        .subscribe(response_pattern)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    let dereg_topic = crate::topic::Topic::try_new(crate::topic::DEREGISTRATION_KEY)
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;
    transport
        .publish(dereg_topic, Envelope::Signal(request_value))
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    let response = tokio::time::timeout(REGISTRATION_TIMEOUT, async {
        response_sub
            .recv_async()
            .await
            .ok()
            .map(|sample| sample.payload().to_bytes().to_vec())
    })
    .await;

    let bytes = match response {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(_) => return Err(RegistrationError::Timeout),
    };

    let resp: RegistrationResponse = serde_json::from_slice(&bytes)
        .map_err(|e| RegistrationError::ServerError(format!("bad deregistration response: {e}")))?;

    match resp.status {
        RegistrationStatus::Ack | RegistrationStatus::Ignore => Ok(()),
        status => Err(RegistrationError::ServerError(format!(
            "unexpected deregistration status: {status:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClientConfig;

    fn test_client_config() -> ClientConfig {
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

    #[test]
    fn registration_envelopes_round_trip_through_json() {
        let req = RegistrationRequest::Register {
            robot_id: "robot-7".to_string(),
            config: Box::new(test_client_config()),
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: RegistrationRequest = serde_json::from_slice(&bytes).unwrap();
        match back {
            RegistrationRequest::Register { robot_id, .. } => assert_eq!(robot_id, "robot-7"),
            _ => panic!("expected a Register request"),
        }

        let resp = RegistrationResponse {
            status: RegistrationStatus::RejectPoisoned,
        };
        let bytes = serde_json::to_vec(&resp).unwrap();
        let back: RegistrationResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.status, RegistrationStatus::RejectPoisoned);
    }
}
