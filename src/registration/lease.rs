//! Registration lease — owns `HashMap<ClientState>` + liveliness token
//! lifecycle semantics. This is the deep interface for state ownership:
//! `register(config) -> Ack|Already|Poisoned` and `on_liveliness(Delete) ->
//! poison iff Registered` with proper lock ordering.
//!
//! The `if state==Registered` check and the `poison` write are performed
//! under a single `write()` lock via [`RegistrationLease::on_liveliness_delete`],
//! fixing the race where a `Delete` arriving between a `read()` and a later
//! `write()` could incorrectly poison a client that had not yet registered
//! or miss a poison for a concurrently registering client.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::{ClientConfig, ServerConfig};

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

/// The lease registry. Owns the `HashMap<ClientState>` and the expected-client
/// configuration. Clones share the same underlying `Arc<RwLock<...>>` so the
/// registration handler and heartbeat monitor can share one instance.
#[derive(Clone)]
pub struct RegistrationLease {
    clients: Arc<RwLock<HashMap<String, ClientEntry>>>,
    config: ServerConfig,
}

/// Backwards-compatible alias — `RegistrationServer` is the same type as
/// `RegistrationLease`. New code should prefer `RegistrationLease`.
pub type RegistrationServer = RegistrationLease;

impl RegistrationLease {
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

    /// Atomically poison `robot_id` iff its current state is `Registered`.
    /// The `if state==Registered` check and the `poison` insertion are
    /// performed under a single `write()` lock, fixing the race present in
    /// the old `if state==Registered { poison }` with two separate locks.
    ///
    /// Returns `true` if the client was poisoned, `false` otherwise (client
    /// was `Unknown`, `Expected`, or already `Poisoned`). Callers (the
    /// heartbeat monitor) should only publish a `Poisoned` alert when this
    /// returns `true`.
    pub async fn on_liveliness_delete(&self, robot_id: &str) -> bool {
        let mut clients = self.clients.write().await;
        match clients.get(robot_id) {
            Some(ClientEntry {
                state: ClientState::Registered,
                ..
            }) => {
                clients.insert(
                    robot_id.to_string(),
                    ClientEntry {
                        state: ClientState::Poisoned,
                        config: None,
                    },
                );
                warn!(robot_id, "client poisoned via liveliness delete");
                true
            }
            _ => false,
        }
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

    fn test_server() -> RegistrationLease {
        RegistrationLease::new(ServerConfig::default())
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

    /// New unit test for ARCH-01: `lease.register → poison → AlreadyRegistered`
    /// without Zenoh. The lease can be tested in-memory; this also exercises
    /// the fixed poison race (`on_liveliness_delete` holds the write lock
    /// across check+poison).
    #[tokio::test]
    async fn lease_register_poison_already_registered() {
        let lease = test_server();
        // register succeeds
        lease
            .register("robot-7", test_client_config())
            .await
            .expect("first register should succeed");
        assert_eq!(lease.state("robot-7").await, ClientState::Registered);

        // liveliness Delete should poison iff Registered — must return true
        let poisoned = lease.on_liveliness_delete("robot-7").await;
        assert!(
            poisoned,
            "on_liveliness_delete should poison a Registered client"
        );
        assert_eq!(lease.state("robot-7").await, ClientState::Poisoned);

        // future registration must be rejected as Poisoned (not Ack / AlreadyRegistered
        // confusion). The issue's shorthand `→ AlreadyRegistered` covers the
        // rejection family; we assert Poisoned here.
        let err = lease
            .register("robot-7", test_client_config())
            .await
            .unwrap_err();
        assert_eq!(err, RegistrationError::Poisoned);

        // deregister after poison must also be rejected as Poisoned
        let err = lease.deregister("robot-7").await.unwrap_err();
        assert_eq!(err, RegistrationError::Poisoned);
    }

    #[tokio::test]
    async fn on_liveliness_delete_does_not_poison_before_register() {
        let lease = test_server();
        // Delete before any registration must NOT poison
        let poisoned = lease.on_liveliness_delete("robot-hb-pre").await;
        assert!(!poisoned, "dropping token before register must NOT poison");
        assert_ne!(lease.state("robot-hb-pre").await, ClientState::Poisoned);

        // subsequent register should still succeed
        lease
            .register("robot-hb-pre", test_client_config())
            .await
            .expect("register after no-op delete should succeed");
        assert_eq!(lease.state("robot-hb-pre").await, ClientState::Registered);
    }

    #[tokio::test]
    async fn on_liveliness_delete_idempotent_after_poison() {
        let lease = test_server();
        lease
            .register("robot-7", test_client_config())
            .await
            .unwrap();
        assert!(lease.on_liveliness_delete("robot-7").await);
        // second delete should not poison again (already Poisoned)
        assert!(!lease.on_liveliness_delete("robot-7").await);
        assert_eq!(lease.state("robot-7").await, ClientState::Poisoned);
    }
}
