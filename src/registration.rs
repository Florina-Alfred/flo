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
    transport: &Transport,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let reg = reg_server.clone();

    let _reg_qable = transport
        .session
        .declare_queryable(REGISTRATION_KEY)
        .callback(move |query| {
            let reg = reg.clone();
            tokio::spawn(async move {
                let bytes = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
                let payload: RegistrationPayload = match serde_json::from_slice(&bytes) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = query.reply(REGISTRATION_KEY, format!("reject:bad_payload:{e}"));
                        return;
                    }
                };
                match reg.register(&payload.robot_id, payload.config).await {
                    Ok(()) => { let _ = query.reply(REGISTRATION_KEY, "ack"); }
                    Err(RegistrationError::AlreadyRegistered) => {
                        let _ = query.reply(REGISTRATION_KEY, "reject:already_registered");
                    }
                    Err(RegistrationError::Poisoned) => {
                        let _ = query.reply(REGISTRATION_KEY, "reject:poisoned");
                    }
                    _ => {}
                }
            });
        })
        .await?;

    let clients_dereg = reg_server.clients.clone();
    let _dereg_qable = transport
        .session
        .declare_queryable(DEREGISTRATION_KEY)
        .callback(move |query| {
            let clients = clients_dereg.clone();
            tokio::spawn(async move {
                let bytes = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
                let robot_id = String::from_utf8_lossy(&bytes).to_string();
                if robot_id.is_empty() {
                    let _ = query.reply(DEREGISTRATION_KEY, "missing robot_id");
                    return;
                }
                let mut clients = clients.write().await;
                if let Some(entry) = clients.get(&robot_id)
                    && entry.state == ClientState::Registered
                {
                    clients.remove(&robot_id);
                    info!(robot_id, "client deregistered");
                    let _ = query.reply(DEREGISTRATION_KEY, "ack");
                    return;
                }
                let _ = query.reply(DEREGISTRATION_KEY, "ignore");
            });
        })
        .await?;

    // Keep the queryables alive.
    std::future::pending::<()>().await;
    Ok(())
}

pub async fn run_heartbeat_monitor(
    transport: &Transport,
    reg_server: RegistrationServer,
) -> zenoh::Result<()> {
    let clients = reg_server.clients;
    let session = transport.session.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, SampleKind)>();

    let _sub = transport
        .session
        .liveliness()
        .declare_subscriber(LIVELINESS_PATTERN)
        .callback(move |sample| {
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
                        let alert_topic =
                            format!("{ALERT_HEARTBEAT_KEY}/{robot_id}");
                        let _ = session.put(alert_topic, "poisoned").await;
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
    transport: &Transport,
    robot_id: &str,
    config: &ClientConfig,
) -> Result<(), RegistrationError> {
    let payload = RegistrationPayload {
        robot_id: robot_id.to_string(),
        config: config.clone(),
    };
    let payload_json = serde_json::to_vec(&payload).map_err(|e| {
        RegistrationError::ServerError(format!("failed to serialize payload: {e}"))
    })?;

    for attempt in 1..=REGISTRATION_RETRIES {
        let replies = transport
            .session
            .get(REGISTRATION_KEY)
            .payload(payload_json.clone())
            .timeout(REGISTRATION_TIMEOUT)
            .await
            .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

        let mut acked = false;
        while let Ok(reply) = replies.recv_async().await {
            match reply.result() {
                Ok(sample) => {
                    let bytes = sample.payload().to_bytes();
                    let text = String::from_utf8_lossy(&bytes);
                    if text == "ack" {
                        acked = true;
                    } else if text.starts_with("reject:already_registered") {
                        return Err(RegistrationError::AlreadyRegistered);
                    } else if text.starts_with("reject:poisoned") {
                        return Err(RegistrationError::Poisoned);
                    } else if text.starts_with("reject:") {
                        return Err(RegistrationError::ServerError(text.to_string()));
                    }
                }
                Err(e) => {
                    warn!(attempt, error = %e, "registration reply error");
                }
            }
        }

        if acked {
            info!(robot_id, "registration successful");
            return Ok(());
        }

        if attempt < REGISTRATION_RETRIES {
            warn!(
                attempt,
                robot_id,
                "registration not acknowledged, retrying..."
            );
            tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_MS * attempt as u64)).await;
        }
    }

    Err(RegistrationError::Timeout)
}

pub async fn deregister_with_server(
    transport: &Transport,
    robot_id: &str,
) -> Result<(), RegistrationError> {
    let replies = transport
        .session
        .get(DEREGISTRATION_KEY)
        .payload(robot_id.as_bytes())
        .timeout(REGISTRATION_TIMEOUT)
        .await
        .map_err(|e| RegistrationError::ServerError(e.to_string()))?;

    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) => {
                let bytes = sample.payload().to_bytes();
                let text = String::from_utf8_lossy(&bytes);
                if text == "ack" {
                    return Ok(());
                }
            }
            Err(e) => {
                warn!(robot_id, error = %e, "deregistration reply error");
            }
        }
    }
    Ok(())
}
