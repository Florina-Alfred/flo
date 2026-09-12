use std::sync::Arc;

use zenoh::Session;
use zenoh::qos::{CongestionControl, Priority, Reliability};

use crate::rules::Qos;
use crate::topic::{Pattern, Topic};

/// Handle to the Zenoh session. A single `Session` multiplexes both QoS classes —
/// QoS is per-put, per the locked decision. The class 1/2 publisher builders below
/// encode the locked QoS knobs; `publish` applies them by QoS class.
///
/// `Transport` is the single low-level seam for the mesh (the one adapter):
/// all publish/subscribe traffic flows through its verbs, so the QoS mapping,
/// the managed-subscription lifecycle, and topic-key ownership stay in one
/// place. `session` is private — callers cannot reach around the seam.
/// `zenoh::Config` and `zenoh::Session` appear only at construction time
/// (`open_with`, `from_session`, `open_router`, `connect_to`) as the documented residual;
/// every other zenoh type is hidden behind the verbs.
pub struct Transport {
    session: Arc<Session>,
    /// Liveliness tokens declared for this client. Held for the session's lifetime
    /// so the token stays declared; dropping it would undeclare the token.
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
}

/// Unified envelope for the single `publish` verb. `Action` carries a QoS class
/// (reliable vs best-effort), `Signal` is the best-effort control plane, and
/// `RawBytes` is for registration payloads that are already serialized.
#[derive(Debug, Clone)]
pub enum Envelope {
    /// Class 1/2 actuator action with explicit QoS.
    Action {
        qos: Qos,
        payload: serde_json::Value,
    },
    /// WebRTC signaling control plane (best-effort JSON).
    Signal(serde_json::Value),
    /// Raw bytes (registration request/response, already serialized).
    RawBytes(Vec<u8>),
}

/// Handle to a managed stream subscription. Dropping it unsubscribes;
/// `recv_async` awaits individual samples. This is the single `subscribe`
/// verb — it replaces `subscribe`, `subscribe_managed`, `subscribe_stream`,
/// and `subscribe_liveliness_managed`. Callers that previously used a callback
/// now spawn a task that loops on `recv_async`.
pub struct Subscription {
    inner: zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>,
}

impl Subscription {
    /// Await the next sample delivered to this subscription.
    pub async fn recv_async(&self) -> zenoh::Result<zenoh::sample::Sample> {
        self.inner.recv_async().await
    }
}

/// Legacy alias — `Subscription` is the single subscription handle.
pub type SubscriptionStream = Subscription;

impl Transport {
    /// Wrap an already-open `zenoh::Session` in a `Transport`.
    pub fn from_session(session: zenoh::Session) -> Self {
        Self {
            session: Arc::new(session),
            _tokens: Vec::new(),
        }
    }

    /// Open a Zenoh session with an explicit config. This is the construction-time
    /// residual where `zenoh::Config` is allowed. Prefer `open_router` / `connect_to`
    /// for new code.
    pub async fn open_with(config: zenoh::Config) -> zenoh::Result<Self> {
        let session = zenoh::open(config).await?;
        Ok(Self::from_session(session))
    }

    /// Open a loopback router for the local demo / tests. Hides `zenoh::Config`
    /// and `insert_json5` behind the seam — callers never touch them.
    pub async fn open_router() -> zenoh::Result<Self> {
        Self::open_with(Self::router_config()).await
    }

    /// Connect as a client to the given explicit endpoints. Hides `zenoh::Config`.
    pub async fn connect_to(endpoints: &[String]) -> zenoh::Result<Self> {
        Self::open_with(Self::client_config(endpoints)).await
    }

    /// Internal: build the zero-config loopback router config.
    fn router_config() -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"router\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "true");
        let _ = c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]");
        c
    }

    /// Internal: build a client-mode config that connects only to the given endpoints.
    fn client_config(endpoints: &[String]) -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"client\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "false");
        if !endpoints.is_empty() {
            // Use serde_json to properly escape endpoints, not string interpolation.
            let endpoints_json =
                serde_json::to_string(endpoints).unwrap_or_else(|_| "[]".to_string());
            let _ = c.insert_json5("connect/endpoints", &endpoints_json);
        }
        c
    }

    /// Apply connect endpoints to an existing config (e.g. an auth-derived config)
    /// without the caller touching `insert_json5`. Used by `runtime` and `flo-client`
    /// to merge `--connect` into the auth config.
    pub fn with_endpoints(mut config: zenoh::Config, endpoints: &[String]) -> zenoh::Config {
        let _ = config.insert_json5("mode", "\"client\"");
        if !endpoints.is_empty() {
            let endpoints_json =
                serde_json::to_string(endpoints).unwrap_or_else(|_| "[]".to_string());
            let _ = config.insert_json5("connect/endpoints", &endpoints_json);
        }
        let _ = config.insert_json5("scouting/multicast/enabled", "false");
        config
    }

    /// Legacy helper for tests that still call `loopback_config` directly.
    /// Prefer `open_router` for new code.
    pub fn loopback_config() -> zenoh::Config {
        Self::router_config()
    }

    /// Legacy helper for tests that still call `connect_config` directly.
    pub fn connect_config(endpoints: &[String]) -> zenoh::Config {
        Self::client_config(endpoints)
    }

    /// Test helper: router config that listens on a specific port with multicast disabled.
    /// Used by `safety_infra06` to isolate router/client pairs. Hides `insert_json5`.
    pub fn test_router_config(port: u16) -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"router\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "false");
        let _ = c.insert_json5("scouting/gossip/enabled", "false");
        let _ = c.insert_json5("listen/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
        c
    }

    /// Test helper: client config that connects to a specific port.
    pub fn test_client_config(port: u16) -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"client\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "false");
        let _ = c.insert_json5("connect/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"));
        c
    }

    /// Open a test router on a specific port.
    pub async fn open_test_router(port: u16) -> zenoh::Result<Self> {
        Self::open_with(Self::test_router_config(port)).await
    }

    /// Open a test client connected to a specific port.
    pub async fn open_test_client(port: u16) -> zenoh::Result<Self> {
        Self::open_with(Self::test_client_config(port)).await
    }

    /// Declare the per-pod liveliness token so the mesh can detect dead clients.
    pub async fn declare_liveliness(&mut self, robot_id: &str) -> zenoh::Result<()> {
        let key = crate::topic::liveliness_key(robot_id);
        let token = self
            .session
            .liveliness()
            .declare_token(key.as_str())
            .await?;
        self._tokens.push(token);
        Ok(())
    }

    /// Return the locators this transport is listening on.
    pub async fn locators(&self) -> Vec<String> {
        self.session
            .info()
            .locators()
            .await
            .into_iter()
            .map(|l| l.to_string())
            .collect()
    }

    /// The single `publish` verb. Callers construct a `Topic` via `topic::` builders
    /// or `Topic::try_new` — a typo fails at construction, not at publish.
    /// The `Envelope` collapses the former `publish` / `publish_signal` / `put_bytes`
    /// variants: `Action` maps to class 1/2 QoS, `Signal` and `RawBytes` are best-effort.
    pub async fn publish(&self, topic: Topic, envelope: Envelope) -> zenoh::Result<()> {
        match envelope {
            Envelope::Action { qos, payload } => {
                let bytes =
                    serde_json::to_vec(&payload).map_err(|e| Box::new(e) as zenoh::Error)?;
                let put = self.session.put(topic.as_str(), bytes);
                let put = match qos {
                    Qos::Reliable => put
                        .reliability(Reliability::Reliable)
                        .congestion_control(CongestionControl::Block)
                        .priority(Priority::InteractiveHigh),
                    Qos::BestEffort => put
                        .reliability(Reliability::BestEffort)
                        .congestion_control(CongestionControl::Drop)
                        .priority(Priority::DataLow),
                };
                put.await.map(|_| ())
            }
            Envelope::Signal(payload) => {
                let bytes =
                    serde_json::to_vec(&payload).map_err(|e| Box::new(e) as zenoh::Error)?;
                self.session.put(topic.as_str(), bytes).await.map(|_| ())
            }
            Envelope::RawBytes(bytes) => self.session.put(topic.as_str(), bytes).await.map(|_| ()),
        }
    }

    /// The single `subscribe` verb. Returns a managed handle that unsubscribes on drop;
    /// `recv_async` awaits samples. This collapses `subscribe`, `subscribe_managed`,
    /// `subscribe_stream`, and `subscribe_liveliness_managed`. Liveliness patterns
    /// (containing `liveliness`) are automatically routed to the liveliness API.
    pub async fn subscribe(&self, pattern: Pattern) -> zenoh::Result<Subscription> {
        let key = pattern.as_str();
        let sub = if key.contains("liveliness") {
            self.session.liveliness().declare_subscriber(key).await?
        } else {
            self.session.declare_subscriber(key).await?
        };
        Ok(Subscription { inner: sub })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::topic;

    #[test]
    fn ruleset_pub_key_has_site_and_name() {
        assert_eq!(
            topic::ruleset_pub_key("cell-7", "acme").as_str(),
            "fleet/cell-7/ruleset/acme"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loopback_transport_round_trips_best_effort() {
        assert_round_trip(
            topic::robot_local("7", "probe"),
            Qos::BestEffort,
            serde_json::json!({"probe": 42}),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loopback_transport_round_trips_reliable() {
        assert_round_trip(
            topic::robot_local("7", "stop"),
            Qos::Reliable,
            serde_json::json!({"stop": true}),
        )
        .await;
    }

    async fn assert_round_trip(topic: Topic, qos: Qos, payload: serde_json::Value) {
        let transport = Transport::open_router()
            .await
            .expect("open loopback transport");

        let pattern = Pattern::try_new(topic.as_str()).expect("topic as pattern");
        let sub = transport
            .subscribe(pattern)
            .await
            .expect("declare subscriber");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Ok(sample) = sub.recv_async().await {
                let _ = tx.send(sample.payload().to_bytes().to_vec());
            }
        });

        transport
            .publish(
                topic.clone(),
                Envelope::Action {
                    qos,
                    payload: payload.clone(),
                },
            )
            .await
            .expect("publish");

        let got = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timeout waiting for sample")
            .expect("channel closed");
        let value: serde_json::Value = serde_json::from_slice(&got).unwrap();
        assert_eq!(value, payload, "round-trip payload mismatch on {topic}");
    }

    #[test]
    fn loopback_config_sets_router_mode_and_localhost_listener() {
        let cfg = Transport::loopback_config();
        assert_eq!(
            cfg.get_json("mode").unwrap(),
            "\"router\"",
            "mode must be router, not the default peer"
        );
        assert_eq!(
            cfg.get_json("scouting/multicast/enabled").unwrap(),
            "true",
            "multicast scouting must be enabled on loopback"
        );
        let endpoints = cfg.get_json("listen/endpoints").unwrap();
        assert!(
            endpoints.contains("tcp/127.0.0.1:0"),
            "missing ephemeral localhost listener, got: {endpoints}"
        );
    }

    #[test]
    fn connect_config_is_client_mode_with_endpoints() {
        let cfg = Transport::connect_config(&[
            "tcp/10.0.0.1:7447".to_string(),
            "tcp/10.0.0.2:7447".to_string(),
        ]);
        assert_eq!(cfg.get_json("mode").unwrap(), "\"client\"");
        let endpoints = cfg.get_json("connect/endpoints").unwrap();
        assert!(
            endpoints.contains("tcp/10.0.0.1:7447"),
            "missing first endpoint, got: {endpoints}"
        );
        assert!(
            endpoints.contains("tcp/10.0.0.2:7447"),
            "missing second endpoint, got: {endpoints}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_managed_subscription_unsubscribes() {
        let transport = Transport::open_router()
            .await
            .expect("open loopback transport");

        let key = topic::robot_local("9", "managed-lifecycle2");
        let pattern = Pattern::try_new(key.as_str()).unwrap();
        let sub = transport
            .subscribe(pattern)
            .await
            .expect("declare subscriber");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        // Spawn forwarder that holds sub; we will abort it to simulate drop-unsubscribe
        let handle = tokio::spawn(async move {
            while let Ok(sample) = sub.recv_async().await {
                let _ = tx.send(sample.payload().to_bytes().to_vec());
            }
        });
        // Abort the forwarder which drops the subscription handle inside it
        handle.abort();
        tokio::time::sleep(Duration::from_millis(500)).await;

        transport
            .publish(
                key,
                Envelope::Action {
                    qos: Qos::BestEffort,
                    payload: serde_json::json!({"x": 1}),
                },
            )
            .await
            .expect("publish");

        let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert!(
            !matches!(got, Ok(Some(_))),
            "dropped subscription must not receive samples; got {got:?}"
        );
    }
}
