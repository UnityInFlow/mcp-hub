#![allow(dead_code)]

pub mod assets;
pub mod routes;
pub mod sse;

use std::sync::Arc;

use axum::{routing::get, Router};
use tokio_util::sync::CancellationToken;

use crate::logs::LogAggregator;
use crate::supervisor::ServerHandle;

/// Shared state for web UI route handlers.
///
/// Holds read-only references to supervisor watch channels and the log
/// aggregator. The web server never mutates server handles -- it only
/// reads snapshots via `state_rx.borrow().clone()`.
pub struct WebState {
    /// Server handles -- cloned Arc from DaemonState for read-only access.
    pub handles: Arc<tokio::sync::Mutex<Vec<ServerHandle>>>,
    /// Log aggregator for SSE streaming and log history.
    pub log_agg: Arc<LogAggregator>,
}

/// Build the Axum router with all web UI routes.
pub fn build_router(state: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(routes::status_page))
        .route("/tools", get(routes::tools_page))
        .route("/logs", get(routes::logs_page))
        .route("/logs/stream", get(sse::log_stream_handler))
        .route("/partials/status", get(routes::status_partial))
        .route(
            "/partials/tools/{server_name}",
            get(routes::tool_detail_partial),
        )
        .route("/health", get(routes::health_handler))
        .route("/static/htmx.min.js", get(assets::serve_htmx))
        .route("/static/htmx-sse.js", get(assets::serve_htmx_sse))
        .route("/static/style.css", get(assets::serve_css))
        .with_state(state)
}

/// Start the web UI server on the given port.
///
/// Spawns an Axum server that listens on 127.0.0.1:{port} and shuts down
/// gracefully when the `shutdown` token is cancelled.
pub async fn start_web_server(
    port: u16,
    state: Arc<WebState>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to bind web UI on port {port}: {e}. \
             Set [hub] web_port = <other> in mcp-hub.toml"
        )
    })?;

    tracing::info!("Web UI listening on http://{addr}");

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}
