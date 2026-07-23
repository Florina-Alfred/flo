use std::sync::Arc;

use zenoh::Session;
use zenoh::qos::{CongestionControl, Priority, Reliability};

use crate::rules::Qos;

/// Stable key-expression namespaces locked in the transport map.
/// Local node traffic stays under `robot/<id>/local/**`; fleet-wide traffic under
/// `fleet/**`; QoS class is marked by `stop/**` (class 1) and `lidar/**` (class 2).
pub const LIVELINESS_KEY: &str = "robot/{id}/client/liveliness";
pub const RULES_KEY: &str = "robot/{id}/local/rules";

/// Fleet-scoped ruleset publish key (PRD §5). Server subscribes here to
/// ingest owner pushes; `{site}` = site id, `{name}` = ruleset_name.
pub const RULESET_PUB_KEY: &str = "fleet/{site}/ruleset/{name}";

/// WebRTC signaling key-expression templates (class-3 video), locked in the
/// webrtc-signaling map. Signaling rides the same zenoh mesh as everything else.
/// `<self>` = this robot's id, `<peer>` = the other robot's id.
pub const SIGNAL_PRESENCE_KEY: &str = "robot/{id}/signal/presence";
pub const SIGNAL_OFFER_KEY: &str = "robot/{self}/signal/{peer}/offer";
pub const SIGNAL_ANSWER_KEY: &str = "robot/{self}/signal/{peer}/answer";
pub const SIGNAL_ICE_KEY: &str = "robot/{self}/signal/{peer}/ice";

/// Handle to the Zenoh session. A single `Session` multiplexes both QoS classes —
/// QoS is per-put, per the locked decision. The class 1/2 publisher builders below
/// encode the locked QoS knobs; `publish` applies them by QoS class.
pub struct Transport {
    pub session: Arc<Session>,
    /// Liveliness tokens declared for this client. Held for the session's lifetime
    /// so the token stays declared; dropping it would undeclare the token.
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
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
    /// production with an auth-derived config.
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
        let _ = c.insert_json5("mode", "\"peer\"");
        let _ = c.insert_json5("scouting/multicast/enabled", "true");
        let _ = c.insert_json5("listen/endpoints/peer", "[\"tcp/127.0.0.1:0\"]");
        c
    }

    /// Declare the per-pod liveliness token so the mesh can detect dead clients.
    /// The token is held inside `Transport` for the session's lifetime.
    pub async fn declare_liveliness(&mut self, robot_id: &str) -> zenoh::Result<()> {
        let key = LIVELINESS_KEY.replace("{id}", robot_id);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruleset_pub_key_has_site_and_name() {
        let k = RULESET_PUB_KEY
            .replace("{site}", "cell-7")
            .replace("{name}", "acme");
        assert_eq!(k, "fleet/cell-7/ruleset/acme");
    }
}
