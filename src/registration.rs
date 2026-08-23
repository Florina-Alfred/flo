use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};
use zenoh::sample::SampleKind;

use crate::config::{ClientConfig, ServerConfig};
use crate::transport::Transport;

const REGISTRATION_RETRIES: u32 = 3;
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_BACKOFF_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    Unknown,
    Expected,
    Registered,
    Poisoned,
}

#[derive(Clone)]
pub struct ClientEntry {
    pub state: ClientState,
    pub config: Option<ClientConfig>,
}

#[derive(Clone)]
pub struct RegistrationServer {
    clients: Arc<RwLock<HashMap<String, ClientEntry>>>,
    config: ServerConfig,
}

impl RegistrationServer {
    pub fn new(config: ServerConfig) -> Self {
        let mut clients = HashMap::new();
        for expected in &config.expected_clients {
            clients.insert(
                expected.robot_id.clone(),
                ClientEntry {
                    state: ClientState::Expected,
                    config: None,
                },
            );
        }
        Self {
            clients: Arc::new(RwLock::new(clients)),
            config,
        }
    }

    pub async fn register(
        &self,
        robot_id: &str,
        config: ClientConfig,
    ) -> Result<(), RegistrationError> {
        let mut clients = self.clients.write().await;
        match clients.get(robot_id) {
            Some(ClientEntry {
                state: ClientState::Poisoned,
                ..
            }) => {
                warn!(robot_id, "registration rejected: client is poisoned");
                Err(RegistrationError::Poisoned)
            }
            Some(ClientEntry {
                state: ClientState::Registered,
                ..
            }) => {
                warn!(robot_id, "registration rejected: already registered");
                Err(RegistrationError::AlreadyRegistered)
            }
            _ => {
                let is_expected = self
                    .config
                    .expected_clients
                    .iter()
                    .any(|e| e.robot_id == robot_id);
                if !is_expected {
                    warn!(
                        robot_id,
                        "registration from unexpected client — accepting with warning"
                    );
                }
                clients.insert(
                    robot_id.to_string(),
                    ClientEntry {
                        state: ClientState::Registered,
                        config: Some(config),
                    },
                );
                info!(robot_id, "client registered");
                Ok(())
            }
        }
    }

    pub async fn deregister(&self, robot_id: &str) -> Result<(), RegistrationError> {
        let mut clients = self.clients.write().await;
        match clients.get(robot_id) {
            Some(ClientEntry {
                state: ClientState::Registered,
                ..
            }) => {
                clients.remove(robot_id);
                info!(robot_id, "client deregistered");
                Ok(())
            }
            Some(ClientEntry {
                state: ClientState::Poisoned,
                ..
            }) => {
                warn!(robot_id, "deregistration ignored: client is poisoned");
                Err(RegistrationError::Poisoned)
            }
            _ => {
                warn!(robot_id, "deregistration ignored: client not registered");
                Err(RegistrationError::NotRegistered)
            }
        }
    }

    pub async fn poison(&self, robot_id: &str) {
        let mut clients = self.clients.write().await;
        clients.insert(
            robot_id.to_string(),
            ClientEntry {
                state: ClientState::Poisoned,
                config: None,
            },
        );
        warn!(robot_id, "client poisoned");
    }

    pub async fn state(&self, robot_id: &str) -> ClientState {
        let clients = self.clients.read().await;
        clients
            .get(robot_id)
            .map(|e| e.state.clone())
            .unwrap_or(ClientState::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    AlreadyRegistered,
    Poisoned,
    NotRegistered,
    Timeout,
    ServerError(String),
}

/// Client → server registration envelope, discriminated by `op`. Replaces the
/// old raw string bodies so a typo or future drift in the wire format surfaces
/// as a deserialize error instead of silently breaking the protocol.
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

/// Serialize a typed response and publish it to `topic` (the registration,
/// deregistration, and heartbeat-alert topics all carry the same envelope).
async fn publish_response(
    transport: &Transport,
    topic: &str,
    status: RegistrationStatus,
) -> Result<(), RegistrationError> {
    let payload = serde_json::to_vec(&RegistrationResponse { status }).map_err(|e| {
        RegistrationError::ServerError(format!("failed to serialize response: {e}"))
    })?;
    transport
        .put_bytes(topic, payload)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))
}

pub async fn run_registration_handler(
    transport: Arc<Transport>,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let reg = reg_server.clone();
    let transport_for_reg = transport.clone();

    let _reg_sub = transport
        .subscribe_managed(crate::topic::REGISTRATION_KEY, move |sample| {
            let reg = reg.clone();
            let transport = transport_for_reg.clone();
            tokio::spawn(async move {
                let bytes = sample.payload().to_bytes();
                let request: RegistrationRequest = match serde_json::from_slice(&bytes) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!("registration: bad request: {e}");
                        return;
                    }
                };
                let RegistrationRequest::Register { robot_id, config } = request else {
                    warn!(
                        "registration: non-register request on {}",
                        crate::topic::REGISTRATION_KEY
                    );
                    return;
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
                let _ = publish_response(&transport, &response_key, status).await;
            });
        })
        .await?;

    info!(
        "registration subscriber active on {}",
        crate::topic::REGISTRATION_KEY
    );

    let dereg_reg = reg_server.clone();
    let transport_for_dereg = transport.clone();
    let _dereg_sub = transport
        .subscribe_managed(crate::topic::DEREGISTRATION_KEY, move |sample| {
            let reg = dereg_reg.clone();
            let transport = transport_for_dereg.clone();
            tokio::spawn(async move {
                let bytes = sample.payload().to_bytes();
                let request: RegistrationRequest = match serde_json::from_slice(&bytes) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!("deregistration: bad request: {e}");
                        return;
                    }
                };
                let RegistrationRequest::Deregister { robot_id } = request else {
                    warn!(
                        "deregistration: non-deregister request on {}",
                        crate::topic::DEREGISTRATION_KEY
                    );
                    return;
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
                let _ = publish_response(&transport, &response_key, status).await;
            });
        })
        .await?;

    std::future::pending::<()>().await;
    Ok(())
}

pub async fn run_heartbeat_monitor(
    transport: Arc<Transport>,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let transport_for_alert = transport.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, SampleKind)>();

    let _sub = transport
        .subscribe_liveliness_managed(crate::topic::LIVELINESS_PATTERN, move |sample| {
            let key = sample.key_expr().to_string();
            let kind = sample.kind();
            let _ = tx.send((key, kind));
        })
        .await?;

    tokio::spawn(async move {
        while let Some((key, kind)) = rx.recv().await {
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
                    // Only poison clients that were actually registered; a token
                    // dropping before registration completes must not reject the
                    // client it is about to register.
                    if reg_server.state(&robot_id).await == ClientState::Registered {
                        reg_server.poison(&robot_id).await;
                        let alert_topic = crate::topic::heartbeat_alert(&robot_id);
                        let _ = publish_response(
                            &transport_for_alert,
                            &alert_topic,
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
    let request_json = serde_json::to_vec(&request)
        .map_err(|e| RegistrationError::ServerError(format!("failed to serialize request: {e}")))?;

    let response_key = crate::topic::registration_response(robot_id);

    for attempt in 1..=REGISTRATION_RETRIES {
        // Subscribe to response topic before sending request.
        let response_sub = transport
            .subscribe_stream(&response_key)
            .await
            .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

        // Send registration request.
        transport
            .put_bytes(crate::topic::REGISTRATION_KEY, request_json.clone())
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

        drop(response_sub);

        if let Ok(Some(bytes)) = response {
            let resp: RegistrationResponse = match serde_json::from_slice(&bytes) {
                Ok(resp) => resp,
                Err(e) => {
                    return Err(RegistrationError::ServerError(format!(
                        "bad registration response: {e}"
                    )));
                }
            };
            return match resp.status {
                RegistrationStatus::Ack => {
                    info!(robot_id, "registration successful");
                    Ok(())
                }
                RegistrationStatus::RejectAlreadyRegistered => {
                    Err(RegistrationError::AlreadyRegistered)
                }
                RegistrationStatus::RejectPoisoned => Err(RegistrationError::Poisoned),
                RegistrationStatus::RejectServerError(msg) => {
                    Err(RegistrationError::ServerError(msg))
                }
                RegistrationStatus::MissingRobotId => {
                    Err(RegistrationError::ServerError("MissingRobotId".to_string()))
                }
                status => Err(RegistrationError::ServerError(format!(
                    "unexpected registration status: {status:?}"
                ))),
            };
        }

        if attempt < REGISTRATION_RETRIES {
            warn!(
                attempt,
                robot_id, "registration not acknowledged, retrying..."
            );
            tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_MS * attempt as u64)).await;
        }
    }

    Err(RegistrationError::Timeout)
}

pub async fn deregister_with_server(
    transport: Arc<Transport>,
    robot_id: &str,
) -> Result<(), RegistrationError> {
    let request = RegistrationRequest::Deregister {
        robot_id: robot_id.to_string(),
    };
    let request_json = serde_json::to_vec(&request)
        .map_err(|e| RegistrationError::ServerError(format!("failed to serialize request: {e}")))?;

    let response_key = crate::topic::deregistration_response(robot_id);

    let response_sub = transport
        .subscribe_stream(&response_key)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    transport
        .put_bytes(crate::topic::DEREGISTRATION_KEY, request_json)
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

    fn test_server() -> RegistrationServer {
        RegistrationServer::new(ServerConfig::default())
    }

    #[tokio::test]
    async fn register_acks_and_state_becomes_registered() {
        let server = test_server();
        server
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        assert_eq!(server.state("robot-7").await, ClientState::Registered);
    }

    #[tokio::test]
    async fn duplicate_register_rejects_already_registered() {
        let server = test_server();
        server
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        let err = server
            .register("robot-7", test_client_config())
            .await
            .unwrap_err();
        assert_eq!(err, RegistrationError::AlreadyRegistered);
    }

    #[tokio::test]
    async fn deregister_after_poison_is_ignored() {
        let server = test_server();
        server
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        server.poison("robot-7").await;
        let err = server.deregister("robot-7").await.unwrap_err();
        assert_eq!(err, RegistrationError::Poisoned);
        assert_eq!(server.state("robot-7").await, ClientState::Poisoned);
    }

    #[tokio::test]
    async fn poison_transition_rejects_future_registration() {
        let server = test_server();
        server
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        server.poison("robot-7").await;
        assert_eq!(server.state("robot-7").await, ClientState::Poisoned);
        let err = server
            .register("robot-7", test_client_config())
            .await
            .unwrap_err();
        assert_eq!(err, RegistrationError::Poisoned);
    }

    #[tokio::test]
    async fn deregister_removes_registered_client() {
        let server = test_server();
        server
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        server.deregister("robot-7").await.unwrap();
        assert_eq!(server.state("robot-7").await, ClientState::Unknown);
    }

    #[tokio::test]
    async fn deregister_unknown_client_is_ignored() {
        let server = test_server();
        let err = server.deregister("ghost").await.unwrap_err();
        assert_eq!(err, RegistrationError::NotRegistered);
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
