#![allow(dead_code)]

pub mod assets;
pub mod routes;
pub mod sse;

use std::sync::Arc;

use axum::{routing::get, Router};
use tokio_util::sync::CancellationToken;

use crate::logs::LogAggregator;
use crate::supervisor::ServerHandle;
use crate::types::ServerSnapshot;

/// Read-only view of one supervised server: its name and snapshot receiver.
///
/// A `watch::Receiver` is cheap to clone and reading it never blocks, so the
/// web UI can serve status without touching the supervisor's `handles` mutex.
#[derive(Clone)]
pub struct ServerView {
    pub name: String,
    pub state_rx: tokio::sync::watch::Receiver<ServerSnapshot>,
}

/// Shared, independently locked list of [`ServerView`]s.
///
/// Writers (startup and config reload) hold the write lock only for the
/// instant it takes to swap the vector, never across an `.await` on process
/// shutdown, so readers are never stalled by a slow reload.
pub type SharedServerViews = Arc<tokio::sync::RwLock<Vec<ServerView>>>;

/// Build a [`SharedServerViews`] from the current supervisor handles.
pub fn new_server_views(handles: &[ServerHandle]) -> SharedServerViews {
    Arc::new(tokio::sync::RwLock::new(views_from_handles(handles)))
}

/// Replace the published views with ones matching `handles`.
///
/// Called after the supervisor's handle list changes (config reload). The
/// views keep serving the previous snapshot set until this swap.
pub async fn publish_server_views(views: &SharedServerViews, handles: &[ServerHandle]) {
    let fresh = views_from_handles(handles);
    *views.write().await = fresh;
}

fn views_from_handles(handles: &[ServerHandle]) -> Vec<ServerView> {
    handles
        .iter()
        .map(|h| ServerView {
            name: h.name.clone(),
            state_rx: h.state_rx.clone(),
        })
        .collect()
}

/// Shared state for web UI route handlers.
///
/// Holds read-only snapshot receivers and the log aggregator. The web server
/// never touches the supervisor's `handles` mutex (which a config reload holds
/// across process shutdowns); it reads `state_rx.borrow().clone()` from its
/// own [`SharedServerViews`].
pub struct WebState {
    /// Snapshot receivers for every supervised server.
    pub servers: SharedServerViews,
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
