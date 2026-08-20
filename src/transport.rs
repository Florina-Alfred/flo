use std::sync::Arc;

use zenoh::Session;
use zenoh::qos::{CongestionControl, Priority, Reliability};

use crate::rules::Qos;

/// Handle to the Zenoh session. A single `Session` multiplexes both QoS classes —
/// QoS is per-put, per the locked decision. The class 1/2 publisher builders below
/// encode the locked QoS knobs; `publish` applies them by QoS class.
///
/// `Transport` is the single low-level seam for the mesh (the one adapter):
/// all publish/subscribe traffic flows through its verbs, so the QoS mapping,
/// the managed-subscription lifecycle, and topic-key ownership stay in one
/// place. `session` is private — callers cannot reach around the seam.
/// `zenoh::Config` and `zenoh::Session` appear only at construction time
/// (`open_with`, `from_session`, `connect_config`) as the documented residual;
/// every other zenoh type is hidden behind the verbs.
pub struct Transport {
    session: Arc<Session>,
    /// Liveliness tokens declared for this client. Held for the session's lifetime
    /// so the token stays declared; dropping it would undeclare the token.
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
}

/// Handle to a managed callback subscription. Dropping it unsubscribes — the
/// managed-subscription lifecycle used for engine sensor topics and the zone
/// tracker. The underlying zenoh subscriber type is hidden behind the seam.
pub struct Subscription {
    // RAII handle: the field is only read by its Drop (unsubscribes on drop).
    #[allow(dead_code)]
    inner: zenoh::pubsub::Subscriber<()>,
}

/// Handle to a managed stream subscription. Dropping it unsubscribes;
/// `recv_async` awaits individual samples (request/response style). The
/// underlying zenoh subscriber type is hidden behind the seam.
pub struct SubscriptionStream {
    inner: zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>,
}

impl SubscriptionStream {
    /// Await the next sample delivered to this subscription.
    pub async fn recv_async(&self) -> zenoh::Result<zenoh::sample::Sample> {
        self.inner.recv_async().await
    }
}

/// Handle to a managed liveliness subscription. Dropping it unsubscribes.
/// Samples arrive as `Put` when a token is declared and `Delete` when it drops,
/// which the heartbeat monitor uses to detect dead clients.
pub struct LivelinessSubscription {
    // RAII handle: the field is only read by its Drop (unsubscribes on drop).
    #[allow(dead_code)]
    inner: zenoh::pubsub::Subscriber<()>,
}

impl Transport {
    /// Wrap an already-open `zenoh::Session` in a `Transport`. Used by the server
    /// mode which opens the session as a router via `zenoh::open` with an auth
    /// config, then wraps the result here.
    pub fn from_session(session: zenoh::Session) -> Self {
        Self {
            session: Arc::new(session),
            _tokens: Vec::new(),
        }
    }

    /// Open a Zenoh session with an explicit config. Used by the local demo to pin
    /// loopback peer discovery (zero-config `cargo run`, no router needed), and by
    /// production with an auth-derived config. `zenoh::Config` is the documented
    /// construction-time residual (see the type docs).
    pub async fn open_with(config: zenoh::Config) -> zenoh::Result<Self> {
        let session = zenoh::open(config).await?;
        Ok(Self::from_session(session))
    }

    /// Build the zero-config loopback config for the local demo: peer mode with
    /// multicast scouting on loopback (auto-meshes multiple `cargo run` on one host)
    /// plus a localhost listen endpoint for robustness on hosts that drop multicast.
    /// `Config::default()` is already a peer; these mutations only harden discovery.
    pub fn loopback_config() -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"router\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "true");
        let _ = c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]");
        c
    }

    /// Build a client-mode config that connects only to the given explicit
    /// endpoints (no multicast scouting, no listen). Used by `flo --connect`.
    pub fn connect_config(endpoints: &[String]) -> zenoh::Config {
        let mut c = zenoh::Config::default();
        let _ = c.insert_json5("mode", "\"client\"");
        if !endpoints.is_empty() {
            let endpoints: Vec<String> = endpoints.iter().map(|e| format!("\"{e}\"")).collect();
            let _ = c.insert_json5("connect/endpoints", &format!("[{}]", endpoints.join(",")));
        }
        c
    }

    /// Declare the per-pod liveliness token so the mesh can detect dead clients.
    /// The token is held inside `Transport` for the session's lifetime.
    pub async fn declare_liveliness(&mut self, robot_id: &str) -> zenoh::Result<()> {
        let key = crate::topic::liveliness_key(robot_id);
        let token = self.session.liveliness().declare_token(&key).await?;
        self._tokens.push(token);
        Ok(())
    }

    /// Publish `payload` to `topic` with the QoS class from the locked decision:
    /// Reliable => class 1 (STOP: Reliable + Block + InteractiveHigh);
    /// BestEffort => class 2 (lidar: BestEffort + Drop + DataLow).
    pub async fn publish(
        &self,
        topic: &str,
        qos: Qos,
        payload: &serde_json::Value,
    ) -> zenoh::Result<()> {
        let bytes = serde_json::to_vec(payload).map_err(|e| Box::new(e) as zenoh::Error)?;
        let put = self.session.put(topic, bytes);
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

    /// Publish arbitrary JSON to a key-expression at best-effort QoS (used for the
    /// WebRTC signaling control plane; not a class 1/2 actuator action).
    /// Named `publish_signal` to distinguish it from the QoS-aware `publish`.
    pub async fn publish_signal(
        &self,
        key_expr: &str,
        payload: &serde_json::Value,
    ) -> zenoh::Result<()> {
        let bytes = serde_json::to_vec(payload).map_err(|e| Box::new(e) as zenoh::Error)?;
        self.session.put(key_expr, bytes).await.map(|_| ())
    }

    /// Publish raw bytes to a key-expression (best-effort, no QoS class). Used by
    /// the registration control plane for request/ack payloads that carry no
    /// actuator class (registration requests, acks, heartbeat alerts).
    pub async fn put_bytes(&self, key_expr: &str, payload: Vec<u8>) -> zenoh::Result<()> {
        self.session.put(key_expr, payload).await.map(|_| ())
    }

    /// Subscribe to a key-expression. The `on_sample` callback runs on Zenoh's
    /// runtime for each received `Sample`; the subscription is kept alive in the
    /// background until the session closes (zenoh owns it after `background()`).
    pub async fn subscribe<F>(&self, key_expr: &str, on_sample: F) -> zenoh::Result<()>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        self.session
            .declare_subscriber(key_expr)
            .callback(on_sample)
            .background()
            .await
    }

    /// Subscribe to a key-expression and return a handle that, when dropped,
    /// unsubscribes (the managed-subscription lifecycle). Useful for subscribers
    /// whose lifecycle must be managed (engine sensor topics, zone tracking).
    pub async fn subscribe_managed<F>(
        &self,
        key_expr: &str,
        on_sample: F,
    ) -> zenoh::Result<Subscription>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let sub = self
            .session
            .declare_subscriber(key_expr)
            .callback(on_sample)
            .await?;
        Ok(Subscription { inner: sub })
    }

    /// Subscribe to a key-expression and return a handle that can be awaited for
    /// individual samples (request/response style). Dropping it unsubscribes.
    pub async fn subscribe_stream(&self, key_expr: &str) -> zenoh::Result<SubscriptionStream> {
        let sub = self.session.declare_subscriber(key_expr).await?;
        Ok(SubscriptionStream { inner: sub })
    }

    /// Subscribe to a liveliness pattern and return a handle that, when dropped,
    /// unsubscribes (the managed-subscription lifecycle). Used by the heartbeat
    /// monitor to observe client liveliness tokens.
    pub async fn subscribe_liveliness_managed<F>(
        &self,
        pattern: &str,
        on_sample: F,
    ) -> zenoh::Result<LivelinessSubscription>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let sub = self
            .session
            .liveliness()
            .declare_subscriber(pattern)
            .callback(on_sample)
            .await?;
        Ok(LivelinessSubscription { inner: sub })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn ruleset_pub_key_has_site_and_name() {
        assert_eq!(
            crate::topic::ruleset_pub_key("cell-7", "acme"),
            "fleet/cell-7/ruleset/acme"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loopback_transport_round_trips_best_effort() {
        assert_round_trip(
            &crate::topic::robot_local("7", "probe"),
            Qos::BestEffort,
            serde_json::json!({"probe": 42}),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loopback_transport_round_trips_reliable() {
        assert_round_trip(
            &crate::topic::robot_local("7", "stop"),
            Qos::Reliable,
            serde_json::json!({"stop": true}),
        )
        .await;
    }

    async fn assert_round_trip(topic: &str, qos: Qos, payload: serde_json::Value) {
        let transport = Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let key = String::from(topic);
        let _sub = transport
            .subscribe_managed(&key, move |s: zenoh::sample::Sample| {
                let _ = tx.send(s.payload().to_bytes().to_vec());
            })
            .await
            .expect("declare subscriber");

        transport
            .publish(topic, qos, &payload)
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
        // The demo config hardens default peer discovery: router mode + multicast
        // scouting on loopback + an ephemeral localhost listener. Assert the
        // mutations landed in the config tree (so distinct sessions mesh).
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
        // The managed-subscription lifecycle: a dropped `Subscription` handle
        // unsubscribes, so later samples never reach the callback. This is the
        // same lifecycle the zone path uses via `subscribe_managed`.
        let transport = Transport::open_with(Transport::loopback_config())
            .await
            .expect("open loopback transport");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let key = crate::topic::robot_local("9", "managed-lifecycle");
        {
            let _sub = transport
                .subscribe_managed(&key, move |s: zenoh::sample::Sample| {
                    let _ = tx.send(s.payload().to_bytes().to_vec());
                })
                .await
                .expect("declare subscriber");
        }

        // Zenoh undeclares asynchronously; give the drop time to propagate before
        // publishing, so a received sample proves the handle was really dropped.
        tokio::time::sleep(Duration::from_millis(200)).await;

        transport
            .publish(&key, Qos::BestEffort, &serde_json::json!({"x": 1}))
            .await
            .expect("publish");

        // Dropping the handle releases the callback (owning `tx`), closing the
        // channel with no samples: `Ok(None)` or a timeout both prove nothing
        // was delivered after the unsubscribe.
        let got = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(
            !matches!(got, Ok(Some(_))),
            "dropped subscription must not receive samples; got {got:?}"
        );
    }
}
