use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Instant;

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

/// Shared observability state for the HTTP server: readiness flag, rule-eval
/// counter, and process start time. All cheap, lock-free atomics.
#[derive(Clone)]
pub struct Health {
    ready: Arc<AtomicBool>,
    eval_total: Arc<AtomicU64>,
    start: Arc<Instant>,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            eval_total: Arc::new(AtomicU64::new(0)),
            start: Arc::new(Instant::now()),
        }
    }
}

impl Health {
    pub fn new() -> Self {
        Self::default()
    }
    /// Mark the client live (Zenoh session + liveliness declared).
    pub fn set_ready(&self) {
        self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    /// Shared handle to the eval counter, for the engine to increment.
    pub fn eval_counter(&self) -> Arc<AtomicU64> {
        self.eval_total.clone()
    }
    /// Prometheus text exposition of the current metrics.
    pub fn metrics_text(&self) -> String {
        let uptime = self.start.elapsed().as_secs_f64();
        let ready = if self.ready.load(std::sync::atomic::Ordering::SeqCst) {
            1
        } else {
            0
        };
        let evals = self.eval_total.load(std::sync::atomic::Ordering::SeqCst);
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

/// Build the health router: `/healthz` (liveness), `/readyz` (readiness),
/// and `/metrics` (Prometheus text exposition).
pub fn router(health: Health) -> Router {
    Router::new()
        .route("/healthz", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/readyz",
            get({
                let health = health.clone();
                move || {
                    let ready = health.ready.load(std::sync::atomic::Ordering::SeqCst);
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
pub async fn serve(health: Health, addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(addr, "health server listening");
    axum::serve(listener, router(health)).await.map_err(|e| {
        tracing::error!(error = %e, "health server failed");
        std::io::Error::other(e)
    })
}
