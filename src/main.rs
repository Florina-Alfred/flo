#![forbid(unsafe_code)]

mod codec;
mod config;
mod device;
mod engine;
mod health;
mod rules;
mod signaling;
mod simulate;
mod transport;
mod video;

use codec::Codec;

use std::sync::Arc;

use tracing::{error, info, warn};

use crate::config::{RuleStore, run_hot_reload};
use crate::health::Health;
use crate::transport::Transport;

/// Minimal CLI: `cargo run` (no args) = local demo. Explicit `--robot-id` /
/// `--config` selects production mode (k8s DaemonSet). Everything else is optional.
struct VideoArgs {
    peer: Option<String>,
    device: Option<String>,
    codec: Codec,
    self_test: bool,
}

impl Default for VideoArgs {
    fn default() -> Self {
        VideoArgs {
            peer: None,
            device: None,
            codec: Codec::H264,
            self_test: false,
        }
    }
}

#[derive(Default)]
struct Args {
    robot_id: Option<String>,
    config: Option<String>,
    simulate: bool,
    simulate_period_ms: u64,
    video: VideoArgs,
    rule: Option<Vec<String>>,
}

fn parse_args() -> Args {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I: Iterator<Item = String>>(mut iter: I) -> Args {
    let mut args = Args::default();
    while let Some(a) = iter.next() {
        if a == "rule" {
            let mut collected: Vec<String> = Vec::new();
            for r in iter.by_ref() {
                collected.push(r);
            }
            args.rule = Some(collected);
            break;
        }
        match a.as_str() {
            "--robot-id" => args.robot_id = iter.next(),
            "--config" => args.config = iter.next(),
            "--simulate" => args.simulate = true,
            "--simulate-period-ms" => {
                args.simulate_period_ms = iter.next().and_then(|v| v.parse().ok()).unwrap_or(1000)
            }
            "--video-peer" => args.video.peer = iter.next().map(|s| s.to_string()),
            "--video-device" => args.video.device = iter.next().map(|s| s.to_string()),
            "--video-codec" => {
                let v = iter.next().unwrap_or_else(|| "h264".to_string());
                args.video.codec = v.parse().unwrap_or_else(|e| panic!("--video-codec: {e}"));
            }
            "--video-self-test" => args.video.self_test = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown arg: {other}"),
        }
    }
    args
}

fn help_text() -> String {
    "flo - robot orchestration client\n\n\
     USAGE:\n\
     \x20\x20flo                        # local demo: simulated sensors + rule engine on loopback zenoh\n\
     \x20\x20flo --robot-id 7           # demo node 7 (open a 2nd terminal with --robot-id 8 to mesh)\n\
     \x20\x20flo --robot-id 7 --config /etc/flo/rules.toml   # production mode (k8s DaemonSet)\n\n\
     OPTIONS:\n\
     \x20\x20--robot-id <id>            robot/node id (also via FLO_ROBOT_ID)\n\
     \x20\x20--config <path>           rules TOML (production); omit for the built-in demo rules\n\
     \x20\x20--simulate                publish synthetic sensor samples (demo input)\n\
     \x20\x20--simulate-period-ms <n>  sensor round interval (default 1000; demo fires 1/s)\n\
     \x20\x20--video-peer <id>         peer robot id to stream WebRTC video to (needs --features media + GStreamer)\n\
     \x20\x20--video-device <path>     video source device (default: synthetic test pattern)\n\
     \x20\x20--video-codec <name>      video codec (default h264)\n\
     \x20\x20--video-self-test         encode-only self-test (no peer needed)\n\
     \x20\x20--help                    this message\n\
     \x20\x20rule check <path>        validate a semantic ruleset (extended TOML) before deploy\n"
        .to_string()
}

fn print_help() {
    println!("{}", help_text());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    health::init_tracing();

    let args = parse_args();

    if let Some(rule_cmd) = &args.rule {
        return run_rule_command(rule_cmd);
    }

    // Demo mode = no explicit production flags. `cargo run` with no args lands here.
    let demo = args.robot_id.is_none() && args.config.is_none();
    let robot_id = args
        .robot_id
        .clone()
        .or_else(|| std::env::var("FLO_ROBOT_ID").ok())
        .unwrap_or_else(|| "7".to_string());

    #[cfg(feature = "media")]
    if args.video.self_test {
        return run_video_self_test(&args.video.codec);
    }

    if demo {
        run_demo(args, robot_id).await?;
    } else {
        run_production(args, robot_id).await?;
    }
    Ok(())
}

#[cfg(feature = "media")]
fn run_video_self_test(_codec: &crate::codec::Codec) -> anyhow::Result<()> {
    use crate::media::{MediaPipeline, SourceSpec};
    let pipeline = MediaPipeline::build(&SourceSpec::Videotest, 1280, 720, 30)?;
    let found = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = found.clone();
    pipeline.start(Box::new(move |bytes: &[u8]| {
        // Annex-B start code: 00 00 00 01
        if bytes.windows(4).any(|w| w == [0x00, 0x00, 0x00, 0x01]) {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::info!(
                len = bytes.len(),
                "▶ encoded H.264 sample (Annex-B start code ok)"
            );
        }
    }))?;
    // Run a few seconds to pull samples.
    std::thread::sleep(std::time::Duration::from_secs(3));
    pipeline.stop();
    anyhow::ensure!(
        found.load(std::sync::atomic::Ordering::SeqCst),
        "no encoded H.264 samples produced"
    );
    println!("SELF-TEST OK: gstreamer encode produced H.264");
    Ok(())
}

/// Demo mode: loopback zenoh, built-in rules, simulated sensors, loud verdicts.
async fn run_demo(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "\n  flo DEMO  —  robot {robot_id} on loopback zenoh\n\
         \x20\x20Simulating sensors and running the rule engine. Watch for '▶ rule fired'.\n\
         \x20\x20Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)\n"
    );

    let mut transport = Transport::open_with(Transport::loopback_config()).await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "demo zenoh session open (loopback peer mesh)");

    let store = RuleStore::bootstrap_demo(&robot_id);

    start_common_subsystems(&transport, &store, &robot_id).await;

    // Simulated sensor input (the demo's fake hardware).
    let sim = args.simulate || true; // demo always simulates unless overridden
    if sim {
        let transport_sim = transport.clone();
        let robot_id_sim = robot_id.clone();
        let period = args.simulate_period_ms.max(100);
        tokio::spawn(async move {
            if let Err(e) = simulate::run_simulate(&transport_sim, &robot_id_sim, period).await {
                error!(error = %e, "simulator exited");
            }
        });
    }

    // Outbound WebRTC video if a peer was requested.
    if let Some(peer) = &args.video.peer {
        let tr = transport.clone();
        let rid = robot_id.clone();
        let pid = peer.clone();
        // Validate the configured video device up front so a bad path fails
        // fast with a clear message instead of an opaque GStreamer error.
        #[cfg_attr(not(feature = "media"), allow(unused_variables))]
        let device = match &args.video.device {
            Some(d) => match crate::device::VideoDevice::validate(d) {
                Ok(dev) => Some(dev),
                Err(e) => {
                    tracing::error!(error = %e, "invalid --video-device, falling back to test pattern");
                    None
                }
            },
            None => None,
        };
        tokio::spawn(async move {
            #[cfg(feature = "media")]
            {
                use crate::media::SourceSpec;
                let source = match device {
                    Some(dev) => dev.to_source_spec(),
                    None => SourceSpec::Videotest,
                };
                if let Err(e) = crate::video::start_video_with_source(&rid, &pid, tr, source).await
                {
                    tracing::error!(error = %e, "video failed");
                }
            }
            #[cfg(not(feature = "media"))]
            {
                if let Err(e) = crate::video::start_video(&rid, &pid, tr).await {
                    tracing::error!(error = %e, "video failed");
                }
            }
        });
    }

    wait_for_subsystems().await;
    Ok(())
}

/// Production mode: file-based rules, optional external config, no simulation.
async fn run_production(
    args: Args,
    robot_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(robot_id, "starting flo client (production mode)");

    let bootstrap = match &args.config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(path, error = %e, "config unreadable -> starting in fail-safe safe-state (no unrestricted motion)");
                safe_state_toml()
            }
        },
        None => "rules = []\n".to_string(),
    };

    // Try semantic (extended-TOML) first; fall back to raw TOML; else safe-state.
    let store = compile_or_fallback(&bootstrap, &robot_id);

    let mut transport = Transport::open().await?;
    transport.declare_liveliness(&robot_id).await?;
    let transport = Arc::new(transport);
    info!(robot_id, "zenoh session open, liveliness declared");

    start_common_subsystems(&transport, &store, &robot_id).await;

    // Optional simulation in production (e.g. a dev node without hardware).
    if args.simulate {
        let transport_sim = transport.clone();
        let robot_id_sim = robot_id.clone();
        let period = args.simulate_period_ms.max(100);
        tokio::spawn(async move {
            if let Err(e) = simulate::run_simulate(&transport_sim, &robot_id_sim, period).await {
                error!(error = %e, "simulator exited");
            }
        });
    }

    // Outbound WebRTC video if a peer was requested.
    if let Some(peer) = &args.video.peer {
        let tr = transport.clone();
        let rid = robot_id.clone();
        let pid = peer.clone();
        // Validate the configured video device up front so a bad path fails
        // fast with a clear message instead of an opaque GStreamer error.
        #[cfg_attr(not(feature = "media"), allow(unused_variables))]
        let device = match &args.video.device {
            Some(d) => match crate::device::VideoDevice::validate(d) {
                Ok(dev) => Some(dev),
                Err(e) => {
                    tracing::error!(error = %e, "invalid --video-device, falling back to test pattern");
                    None
                }
            },
            None => None,
        };
        tokio::spawn(async move {
            #[cfg(feature = "media")]
            {
                use crate::media::SourceSpec;
                let source = match device {
                    Some(dev) => dev.to_source_spec(),
                    None => SourceSpec::Videotest,
                };
                if let Err(e) = crate::video::start_video_with_source(&rid, &pid, tr, source).await
                {
                    tracing::error!(error = %e, "video failed");
                }
            }
            #[cfg(not(feature = "media"))]
            {
                if let Err(e) = crate::video::start_video(&rid, &pid, tr).await {
                    tracing::error!(error = %e, "video failed");
                }
            }
        });
    }

    wait_for_subsystems().await;
    Ok(())
}

/// A minimal fail-safe ruleset: no motion commands are emitted.
fn safe_state_toml() -> String {
    "rules = []\n".to_string()
}

/// Compile extended-TOML if it parses as semantic; otherwise treat as raw TOML.
/// On any failure, fall back to a fail-safe empty ruleset.
fn compile_or_fallback(text: &str, robot_id: &str) -> RuleStore {
    if let Ok(doc) = flo_rs::semantic::parse_semantic(text) {
        match flo_rs::semantic::compile(&doc, robot_id) {
            Ok(rules) => match RuleStore::bootstrap(&rules.to_toml()) {
                Ok(s) => return s,
                Err(e) => {
                    tracing::error!(error = %e, "semantic compile produced invalid rules -> safe-state")
                }
            },
            Err(e) => tracing::error!(error = %e, "semantic validation failed -> safe-state"),
        }
    }
    match RuleStore::bootstrap(text) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "config invalid -> starting in fail-safe safe-state");
            RuleStore::bootstrap(&safe_state_toml()).expect("safe-state always parses")
        }
    }
}

/// Start health server, hot-reload, rule engine, and WebRTC signaling. Shared by
/// both demo and production modes (the only difference is input + rules source).
async fn start_common_subsystems(transport: &Arc<Transport>, store: &RuleStore, robot_id: &str) {
    let health = Health::new();

    let health_task = {
        let health = health.clone();
        tokio::spawn(async move {
            if let Err(e) = health::serve(health, "0.0.0.0:8080").await {
                error!(error = %e, "health server exited");
            }
        })
    };

    let reload_task = {
        let transport = transport.clone();
        let store = store.clone();
        let robot_id = robot_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = run_hot_reload(&transport, &robot_id, store).await {
                error!(error = %e, "hot-reload subscriber exited");
            }
        })
    };

    let engine_task = {
        let transport = transport.clone();
        let store = store.clone();
        let eval_counter = health.eval_counter();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(transport, store, eval_counter).await {
                error!(error = %e, "rule engine exited");
            }
        })
    };

    let signal_task = {
        let transport = transport.clone();
        let robot_id = robot_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = run_signaling(transport.clone(), &robot_id).await {
                error!(error = %e, "signaling exited");
            }
        })
    };

    health.set_ready();
    info!("flo ready");

    // Store handles so they live for the process; tasks are joined by the caller.
    let _ = (health_task, reload_task, engine_task, signal_task);
}

async fn run_signaling(transport: std::sync::Arc<Transport>, robot_id: &str) -> zenoh::Result<()> {
    signaling::publish_presence(
        &transport,
        robot_id,
        vec![format!("robot/{}/local/cam0", robot_id)],
    )
    .await?;
    signaling::subscribe_presence(&transport, |p: signaling::Presence| {
        info!(peer = %p.id, streams = ?p.streams, "discovered peer");
    })
    .await?;
    // The mesh handler auto-answers inbound offers so connectivity is two-way:
    // whichever peer initiates, the other side establishes its own PeerConnection
    // and streams media back.
    let handler = MeshSignalHandler::new(robot_id, transport.clone());
    signaling::run_signal_receiver(&transport, robot_id, handler).await
}

/// Signal handler for the always-on mesh listener. Unlike the one-shot
/// [`crate::video::start_video`] initiator, this answers inbound offers from any
/// peer by lazily creating a `VideoPeer` per peer and delegating to its
/// `SignalHandler` impl. This is what makes WebRTC connectivity bidirectional.
#[derive(Clone)]
struct MeshSignalHandler {
    inner: std::sync::Arc<MeshSignalHandlerInner>,
}

struct MeshSignalHandlerInner {
    robot_id: String,
    transport: std::sync::Arc<Transport>,
    /// One answering PeerConnection per remote peer. Created on first inbound
    /// offer; reused for subsequent signaling with that peer.
    peers: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<crate::video::VideoPeer>>,
    >,
}

impl MeshSignalHandler {
    fn new(robot_id: &str, transport: std::sync::Arc<Transport>) -> Self {
        Self {
            inner: std::sync::Arc::new(MeshSignalHandlerInner {
                robot_id: robot_id.to_string(),
                transport,
                peers: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
        }
    }

    /// Get the existing answering peer for `from`, or create one. The creation
    /// await happens outside the lock to avoid blocking other signaling.
    async fn peer_for(&self, from: &str) -> Option<std::sync::Arc<crate::video::VideoPeer>> {
        if let Some(p) = self.inner.peers.lock().unwrap().get(from) {
            return Some(p.clone());
        }
        let peer = match crate::video::VideoPeer::answer(
            &self.inner.robot_id,
            from,
            self.inner.transport.clone(),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, peer = from, "failed to create answering PeerConnection");
                return None;
            }
        };
        let mut g = self.inner.peers.lock().unwrap();
        Some(g.entry(from.to_string()).or_insert(peer).clone())
    }
}

impl signaling::SignalHandler for MeshSignalHandler {
    fn on_offer(&self, from: &str, msg: &signaling::SignalMessage) {
        let h = self.clone();
        let from = from.to_string();
        let msg = msg.clone();
        tokio::spawn(async move {
            if let Some(peer) = h.peer_for(&from).await {
                peer.on_offer(&from, &msg);
            }
        });
    }
    fn on_answer(&self, from: &str, msg: &signaling::SignalMessage) {
        let h = self.clone();
        let from = from.to_string();
        let msg = msg.clone();
        tokio::spawn(async move {
            if let Some(peer) = h.inner.peers.lock().unwrap().get(&from).cloned() {
                peer.on_answer(&from, &msg);
            }
        });
    }
    fn on_ice(&self, from: &str, candidate: &signaling::IceCandidate) {
        let h = self.clone();
        let from = from.to_string();
        let candidate = candidate.clone();
        tokio::spawn(async move {
            if let Some(peer) = h.inner.peers.lock().unwrap().get(&from).cloned() {
                peer.on_ice(&from, &candidate);
            }
        });
    }
}

/// Run until any subsystem dies (k8s / process supervisor restarts).
async fn wait_for_subsystems() {
    // The spawned tasks own the long-lived work; this future just idles. A real
    // deployment would `tokio::select!` on the JoinHandles. For the demo we block
    // so `cargo run` stays alive and visible.
    std::future::pending::<()>().await;
}

fn run_rule_command(cmd: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd.first().map(String::as_str) {
        Some("check") => {
            let path = cmd.get(1).ok_or("usage: flo rule check <path>")?;
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            match flo_rs::semantic::parse_semantic(&text) {
                Ok(doc) => match flo_rs::semantic::validate(&doc) {
                    Ok(()) => {
                        println!("OK: {path} is a valid semantic ruleset");
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("INVALID: {e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("PARSE ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown rule subcommand: {other:?} (try 'flo rule check <path>')");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_args() {
        let args = parse_args_from(
            [
                "flo",
                "--robot-id",
                "7",
                "--video-peer",
                "8",
                "--video-device",
                "/dev/video0",
                "--video-codec",
                "h264",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(args.robot_id.as_deref(), Some("7"));
        assert_eq!(args.video.peer.as_deref(), Some("8"));
        assert_eq!(args.video.device.as_deref(), Some("/dev/video0"));
        assert_eq!(args.video.codec, crate::codec::Codec::H264);
    }

    #[test]
    fn rejects_unknown_codec() {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_args_from(
                ["flo", "--video-codec", "vp8"]
                    .into_iter()
                    .map(String::from),
            )
        }));
        assert!(r.is_err());
    }

    /// End-to-end proof that the always-on mesh listener actually answers an
    /// inbound offer (the two-way half of WebRTC connectivity): open a loopback
    /// transport, attach `MeshSignalHandler`, publish an offer from a fake peer,
    /// and assert an answer is published back on the expected key-expr.
    #[tokio::test(flavor = "multi_thread")]
    async fn mesh_handler_answers_inbound_offer() {
        use std::sync::Arc;

        // Roles: `offerer` (robot7) opens a real PeerConnection and publishes a
        // valid offer; `answerer` (peer8) hosts the always-on MeshSignalHandler
        // which must auto-create its own PeerConnection and publish an answer.
        let offerer = "robot7";
        let answerer = "peer8";
        let transport = Arc::new(
            Transport::open_with(Transport::loopback_config())
                .await
                .expect("open loopback transport"),
        );

        // The answerer side: always-on mesh listener.
        let handler = MeshSignalHandler::new(answerer, transport.clone());
        crate::signaling::run_signal_receiver(&transport, answerer, handler)
            .await
            .expect("signal receiver");

        // Subscribe to the answer the answerer should publish back.
        // Key layout: robot/{answerer}/signal/{offerer}/answer.
        let answer_key = format!("robot/{answerer}/signal/{offerer}/answer");
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        transport
            .subscribe(&answer_key, move |s: zenoh::sample::Sample| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(s.payload().to_bytes().to_vec());
                }
            })
            .await
            .expect("subscribe answer key");

        // The offerer side: a real PeerConnection producing a valid SDP offer.
        // `VideoPeer::offer` publishes the offer over the same transport, which
        // the answerer's mesh listener receives.
        let _offerer = crate::video::VideoPeer::offer(offerer, answerer, transport.clone())
            .await
            .expect("offerer PeerConnection");

        // The handler must auto-create an answering PeerConnection and publish
        // an answer within a few seconds.
        let got = tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .expect("answer within 15s")
            .expect("answer payload");
        let v: serde_json::Value = serde_json::from_slice(&got).expect("answer is JSON");
        assert_eq!(v["kind"], "answer");
        assert_eq!(v["from"], answerer);
        assert_eq!(v["to"], offerer);
    }
}
