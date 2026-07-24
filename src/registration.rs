use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::{ClientConfig, ServerConfig};
use crate::transport::Transport;

const REGISTRATION_KEY: &str = "fleet/registration";
const DEREGISTRATION_KEY: &str = "fleet/deregistration";
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
    let clients = reg_server.clients.clone();

    let _reg_qable = transport
        .session
        .declare_queryable(REGISTRATION_KEY)
        .callback(move |query| {
            let clients = clients.clone();
            tokio::spawn(async move {
                let robot_id = query.selector().key_expr().to_string();
                let payload = query.payload();
                let payload_bytes = payload.map(|p| p.to_bytes()).unwrap_or_default();
                let config_str = String::from_utf8_lossy(&payload_bytes);

                match ClientConfig::from_toml(&config_str) {
                    Ok(client_config) => {
                        let result = async {
                            let mut clients = clients.write().await;
                            if let Some(entry) = clients.get(&robot_id) {
                                match entry.state {
                                    ClientState::Poisoned => {
                                        return Err(RegistrationError::Poisoned);
                                    }
                                    ClientState::Registered => {
                                        return Err(RegistrationError::AlreadyRegistered);
                                    }
                                    _ => {}
                                }
                            }
                            clients.insert(
                                robot_id.clone(),
                                ClientEntry {
                                    state: ClientState::Registered,
                                    config: Some(client_config),
                                },
                            );
                            Ok(())
                        }
                        .await;
                        match result {
                            Ok(()) => {
                                let _ = query.reply(REGISTRATION_KEY, "ack");
                            }
                            Err(RegistrationError::AlreadyRegistered) => {
                                let _ = query.reply(REGISTRATION_KEY, "reject:already_registered");
                            }
                            Err(RegistrationError::Poisoned) => {
                                let _ = query.reply(REGISTRATION_KEY, "reject:poisoned");
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        let _ = query.reply(REGISTRATION_KEY, format!("reject:invalid_config:{e}"));
                    }
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
                let robot_id = query.selector().key_expr().to_string();
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

pub async fn register_with_client(
    transport: &Transport,
    robot_id: &str,
    config: &ClientConfig,
) -> Result<(), RegistrationError> {
    let config_toml = toml::to_string(config).map_err(|e| {
        RegistrationError::ServerError(format!("failed to serialize config: {e}"))
    })?;

    for attempt in 1..=REGISTRATION_RETRIES {
        let replies = transport
            .session
            .get(REGISTRATION_KEY)
            .payload(config_toml.as_bytes())
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
