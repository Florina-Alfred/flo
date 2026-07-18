use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tracing::info;

/// Initialize structured logging (fmt + env-filter, JSON when `FLO_JSON_LOGS=1`).
pub fn init_tracing() {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        );
    if std::env::var("FLO_JSON_LOGS").is_ok() {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Shared readiness flag. Set true once the Zenoh session and liveliness token are
/// declared; `/readyz` reflects it so k8s delays traffic until the client is live.
#[derive(Clone, Default)]
pub struct Health {
    ready: Arc<std::sync::atomic::AtomicBool>,
}

impl Health {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_ready(&self) {
        self.ready
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn is_ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Build the health router: `/healthz` (liveness) and `/readyz` (readiness).
pub fn router(health: Health) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { axum::http::StatusCode::OK }),
        )
        .route(
            "/readyz",
            get(move || {
                let health = health.clone();
                async move {
                    if health.is_ready() {
                        axum::http::StatusCode::OK
                    } else {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }),
        )
}

/// Serve the health router on the given address (e.g. `0.0.0.0:8080`).
pub async fn serve(health: Health, addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(addr, "health server listening");
    axum::serve(listener, router(health))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "health server failed");
            std::io::Error::other(e)
        })
}
