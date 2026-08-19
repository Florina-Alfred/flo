use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};
use zenoh::sample::SampleKind;

use crate::config::{ClientConfig, ServerConfig};
use crate::transport::Transport;

const REGISTRATION_KEY: &str = "fleet/registration";
const DEREGISTRATION_KEY: &str = "fleet/deregistration";
const LIVELINESS_PATTERN: &str = "robot/*/client/liveliness";
const ALERT_HEARTBEAT_KEY: &str = "fleet/alerts/heartbeat";
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

    pub async fn deregister(&self, robot_id: &str) -> Result<(), String> {
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
                Ok(())
            }
            _ => {
                warn!(robot_id, "deregistration ignored: unknown client");
                Ok(())
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

#[derive(Debug)]
pub enum RegistrationError {
    AlreadyRegistered,
    Poisoned,
    Timeout,
    ServerError(String),
}

pub async fn run_registration_handler(
    transport: Arc<Transport>,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let reg = reg_server.clone();
    let transport_for_reg = transport.clone();

    let _reg_sub = transport
        .subscribe_managed(REGISTRATION_KEY, move |sample| {
            let reg = reg.clone();
            let transport = transport_for_reg.clone();
            tokio::spawn(async move {
                let bytes = sample.payload().to_bytes();
                let payload: RegistrationPayload = match serde_json::from_slice(&bytes) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("registration: bad payload: {e}");
                        return;
                    }
                };
                let response_key = format!("{}/response/{}", REGISTRATION_KEY, payload.robot_id);
                match reg.register(&payload.robot_id, payload.config).await {
                    Ok(()) => {
                        let _ = transport.put_bytes(&response_key, b"ack".to_vec()).await;
                    }
                    Err(RegistrationError::AlreadyRegistered) => {
                        let _ = transport
                            .put_bytes(&response_key, b"reject:already_registered".to_vec())
                            .await;
                    }
                    Err(RegistrationError::Poisoned) => {
                        let _ = transport
                            .put_bytes(&response_key, b"reject:poisoned".to_vec())
                            .await;
                    }
                    _ => {}
                }
            });
        })
        .await?;

    info!("registration subscriber active on {REGISTRATION_KEY}");

    let clients_dereg = reg_server.clients.clone();
    let transport_for_dereg = transport.clone();
    let _dereg_sub = transport
        .subscribe_managed(DEREGISTRATION_KEY, move |sample| {
            let clients = clients_dereg.clone();
            let transport = transport_for_dereg.clone();
            tokio::spawn(async move {
                let robot_id = String::from_utf8_lossy(&sample.payload().to_bytes()).to_string();
                let response_key = format!("{}/response/{}", DEREGISTRATION_KEY, robot_id);
                if robot_id.is_empty() {
                    let _ = transport
                        .put_bytes(&response_key, b"missing robot_id".to_vec())
                        .await;
                    return;
                }
                let mut clients = clients.write().await;
                if let Some(entry) = clients.get(&robot_id)
                    && entry.state == ClientState::Registered
                {
                    clients.remove(&robot_id);
                    info!(robot_id, "client deregistered");
                    let _ = transport.put_bytes(&response_key, b"ack".to_vec()).await;
                    return;
                }
                let _ = transport.put_bytes(&response_key, b"ignore".to_vec()).await;
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
    let clients = reg_server.clients;
    let transport_for_alert = transport.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, SampleKind)>();

    let _sub = transport
        .subscribe_liveliness_managed(LIVELINESS_PATTERN, move |sample| {
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
                    let mut w = clients.write().await;
                    if let Some(entry) = w.get(&robot_id)
                        && entry.state == ClientState::Registered
                    {
                        warn!(%robot_id, "heartbeat: client disconnected unexpectedly — poisoning");
                        w.insert(
                            robot_id.clone(),
                            ClientEntry {
                                state: ClientState::Poisoned,
                                config: None,
                            },
                        );
                        let alert_topic = format!("{ALERT_HEARTBEAT_KEY}/{robot_id}");
                        let _ = transport_for_alert
                            .put_bytes(&alert_topic, b"poisoned".to_vec())
                            .await;
                    }
                }
            }
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationPayload {
    pub robot_id: String,
    pub config: ClientConfig,
}

pub async fn register_with_client(
    transport: Arc<Transport>,
    robot_id: &str,
    config: &ClientConfig,
) -> Result<(), RegistrationError> {
    let payload = RegistrationPayload {
        robot_id: robot_id.to_string(),
        config: config.clone(),
    };
    let payload_json = serde_json::to_vec(&payload)
        .map_err(|e| RegistrationError::ServerError(format!("failed to serialize payload: {e}")))?;

    let response_key = format!("{}/response/{}", REGISTRATION_KEY, robot_id);

    for attempt in 1..=REGISTRATION_RETRIES {
        // Subscribe to response topic before sending request.
        let response_sub = transport
            .subscribe_stream(&response_key)
            .await
            .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

        // Send registration request.
        transport
            .put_bytes(REGISTRATION_KEY, payload_json.clone())
            .await
            .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

        // Wait for response with timeout.
        let response = tokio::time::timeout(REGISTRATION_TIMEOUT, async {
            response_sub
                .recv_async()
                .await
                .ok()
                .map(|sample| String::from_utf8_lossy(&sample.payload().to_bytes()).to_string())
        })
        .await;

        drop(response_sub);

        match response {
            Ok(Some(text)) if text == "ack" => {
                info!(robot_id, "registration successful");
                return Ok(());
            }
            Ok(Some(text)) if text.starts_with("reject:already_registered") => {
                return Err(RegistrationError::AlreadyRegistered);
            }
            Ok(Some(text)) if text.starts_with("reject:poisoned") => {
                return Err(RegistrationError::Poisoned);
            }
            Ok(Some(text)) => {
                return Err(RegistrationError::ServerError(text));
            }
            Ok(None) | Err(_) => {}
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
    let response_key = format!("{}/response/{}", DEREGISTRATION_KEY, robot_id);

    let response_sub = transport
        .subscribe_stream(&response_key)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    transport
        .put_bytes(DEREGISTRATION_KEY, robot_id.as_bytes().to_vec())
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    let response = tokio::time::timeout(REGISTRATION_TIMEOUT, async {
        response_sub
            .recv_async()
            .await
            .ok()
            .map(|sample| String::from_utf8_lossy(&sample.payload().to_bytes()).to_string())
    })
    .await;

    match response {
        Ok(Some(text)) if text == "ack" => Ok(()),
        _ => Ok(()),
    }
}
