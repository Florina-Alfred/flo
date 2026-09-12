use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;
use tracing::info;

/// Initialize structured logging (fmt + env-filter, JSON when `FLO_JSON_LOGS=1`).
pub fn init_tracing() {
    let builder = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    );
    if std::env::var("FLO_JSON_LOGS").is_ok() {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Shared observability + readiness gate. ReadyGate is the single readiness
/// interface: the engine consumes the token and flips it once its sensor
/// subscriptions are live; the HTTP server reports it on `/readyz`. Health is
/// kept as a type alias for backwards compatibility.
///
/// All fields are cheap, lock-free atomics behind `Arc` so the gate can be
/// cloned cheaply between the health server and the engine.
#[derive(Clone)]
pub struct ReadyGate {
    ready: Arc<AtomicBool>,
    eval_total: Arc<AtomicU64>,
    start: Arc<Instant>,
}

impl Default for ReadyGate {
    fn default() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            eval_total: Arc::new(AtomicU64::new(0)),
            start: Arc::new(Instant::now()),
        }
    }
}

impl ReadyGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the process ready. Crate-private: only the engine flips readiness
    /// after its subscriptions are live, so readiness has a single writer.
    pub(crate) fn set_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Whether readiness has been confirmed (what `/readyz` reports).
    /// Crate-private: external callers observe via the HTTP probe, not direct
    /// flag reads. Tests within the crate may still call it.
    pub(crate) fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Public read for integration tests and external observers that need to
    /// poll readiness without HTTP. Kept `pub` for supervision tests that
    /// previously used `Health::is_ready`; new code should prefer the HTTP
    /// `/readyz` probe.
    pub fn is_ready_public(&self) -> bool {
        self.is_ready()
    }

    /// Shared handle to the eval counter, for the engine to increment.
    pub fn eval_counter(&self) -> Arc<AtomicU64> {
        self.eval_total.clone()
    }

    /// Prometheus text exposition of the current metrics.
    pub fn metrics_text(&self) -> String {
        let uptime = self.start.elapsed().as_secs_f64();
        let ready = if self.ready.load(Ordering::SeqCst) {
            1
        } else {
            0
        };
        let evals = self.eval_total.load(Ordering::SeqCst);
        format!(
            "# HELP flo_uptime_seconds seconds since the process started\n\
             # TYPE flo_uptime_seconds gauge\n\
             flo_uptime_seconds {uptime:.3}\n\
             # HELP flo_process_ready 1 once flo has started and declared its subsystems (ready probe == 200)\n\
             # TYPE flo_process_ready gauge\n\
             flo_process_ready {ready}\n\
             # HELP flo_rule_eval_total cumulative 50ms engine re-evaluation ticks\n\
             # TYPE flo_rule_eval_total counter\n\
             flo_rule_eval_total {evals}\n"
        )
    }
}

/// Backwards-compatible alias: `Health` is now `ReadyGate`.
pub type Health = ReadyGate;

/// Build the health router: `/healthz` (liveness), `/readyz` (readiness),
/// and `/metrics` (Prometheus text exposition).
pub fn router(health: ReadyGate) -> Router {
    Router::new()
        .route("/healthz", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/readyz",
            get({
                let health = health.clone();
                move || {
                    let ready = health.ready.load(Ordering::SeqCst);
                    async move {
                        if ready {
                            axum::http::StatusCode::OK
                        } else {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        }
                    }
                }
            }),
        )
        .route(
            "/metrics",
            get({
                let health = health.clone();
                move || {
                    let text = health.metrics_text();
                    async move {
                        (
                            [(
                                axum::http::header::CONTENT_TYPE,
                                "text/plain; version=0.0.4",
                            )],
                            text,
                        )
                    }
                }
            }),
        )
}

/// Serve the health router on the given address (e.g. `0.0.0.0:8080`).
/// Use `"0.0.0.0:0"` to let the OS assign a random port.
pub async fn serve(health: ReadyGate, addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let actual = listener.local_addr()?;
    info!(addr = %actual, "health server listening");
    axum::serve(listener, router(health)).await.map_err(|e| {
        tracing::error!(error = %e, "health server failed");
        std::io::Error::other(e)
    })
}

/// One-shot liveness probe for container `HEALTHCHECK`s (distroless images have
/// no shell/curl). Converts a bind address like `0.0.0.0:8080` into a loopback
/// TCP connect and returns `true` only on an `HTTP 200` from `/healthz`.
///
/// Uses a plain blocking `TcpStream` so it works in tiny images without an HTTP
/// client library. The socket timeout bounds the window the probe waits.
pub fn probe(addr: &str) -> bool {
    let Some((host, port)) = addr.rsplit_once(':') else {
        return false;
    };
    let Ok(port): Result<u16, _> = port.parse() else {
        return false;
    };
    // Bind-time `0.0.0.0`/wildcard listens on all interfaces → probe loopback.
    let target = match host {
        "0.0.0.0" => "127.0.0.1",
        other => other,
    };

    let mut stream = match std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::new(
            target
                .parse()
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            port,
        ),
        Duration::from_secs(2),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

    let request =
        format!("GET /healthz HTTP/1.1\r\nHost: {target}:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = [0u8; 512];
    let mut received = Vec::new();
    loop {
        match stream.read(&mut response) {
            Ok(0) | Err(_) => break,
            Ok(n) => received.extend_from_slice(&response[..n]),
        }
        if received.contains(&b'\n') {
            break;
        }
        if received.len() >= 512 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&received);
    head.lines()
        .next()
        .map(|line| line.starts_with("HTTP/1.1 200"))
        .unwrap_or(false)
}

/// Single supervisor for both binaries. `await_shutdown` waits for the first
/// supervised subsystem to exit and treats any completion as fatal, so health
/// is never hidden behind a `health_task.await.ok(); Ok(())` shim.
#[derive(Debug)]
pub struct Supervisor;

impl Supervisor {
    /// Wait for the first subsystem in `handles` to complete. Any completion —
    /// clean exit or panic — is logged as fatal and returned as an error so a
    /// process supervisor can restart the binary. Health is always supervised.
    pub async fn await_shutdown(
        handles: Vec<(&'static str, tokio::task::JoinHandle<()>)>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if handles.is_empty() {
            return Ok(());
        }
        let mut set = tokio::task::JoinSet::new();
        for (name, handle) in handles {
            set.spawn(async move {
                let res = handle.await;
                (name, res)
            });
        }
        if let Some(joined) = set.join_next().await {
            match joined {
                Ok((name, res)) => return Self::fatal_exit(name, res),
                Err(join_err) => {
                    tracing::error!(error = %join_err, "supervisor wrapper panicked");
                    return Err(format!("supervisor wrapper panicked: {join_err:?}").into());
                }
            }
        }
        Ok(())
    }

    fn fatal_exit(
        subsystem: &str,
        res: Result<(), tokio::task::JoinError>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::error!(
            subsystem = subsystem,
            "fatal: {subsystem} subsystem exited: {res:?}"
        );
        Err(format!("{subsystem} subsystem exited: {res:?}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn probe_hits_healthz_and_returns_ok() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("0.0.0.0:{}", listener.local_addr().unwrap().port());
        let serve = tokio::spawn(async move {
            axum::serve(listener, router(Health::new())).await.unwrap();
        });

        // Poll for readiness instead of sleeping a fixed amount (which flaked
        // under load: the server may not be bound when probes start). Bounded
        // by a hard deadline so a genuinely broken server still fails fast.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if probe(&addr) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "health server did not become reachable in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        serve.abort();
    }

    #[test]
    fn probe_fails_when_nothing_listens() {
        assert!(!probe("127.0.0.1:9999"));
    }

    #[test]
    fn probe_rejects_malformed_addr() {
        assert!(!probe("not-an-addr"));
        assert!(!probe("127.0.0.1:notaport"));
    }
}
