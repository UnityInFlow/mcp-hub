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
    /// The port the web UI is bound to; the `Host` header check requires it.
    pub port: u16,
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
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            host_guard,
        ))
        .with_state(state)
}

/// Reject requests whose `Host` is not a loopback name on the web UI port.
///
/// Binding to 127.0.0.1 does not stop a DNS-rebinding page from reaching the
/// UI (the browser connects to loopback but sends the attacker's `Host`), so
/// the allowlist is `localhost`, `127.0.0.1` and `[::1]`, with or without
/// `:<web_port>`. A missing `Host` (HTTP/1.0 tools) is allowed: browsers
/// always send one, and a rebinding attack runs in a browser.
async fn host_guard(
    axum::extract::State(state): axum::extract::State<Arc<WebState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let allowed = match req.headers().get(axum::http::header::HOST) {
        None => true,
        Some(value) => value
            .to_str()
            .map(|h| host_allowed(h, state.port))
            .unwrap_or(false),
    };
    if allowed {
        next.run(req).await
    } else {
        (
            axum::http::StatusCode::FORBIDDEN,
            "403 Forbidden: unexpected Host header (mcp-hub web UI only answers on localhost)\n",
        )
            .into_response()
    }
}

/// Whether a `Host` header value names a loopback host on `port` (or no port).
fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    // Split "name[:port]" -- an IPv6 literal keeps its colons inside brackets.
    let (name, port_part) = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((inner, tail)) => (format!("[{inner}]"), tail.strip_prefix(':')),
            None => return false,
        },
        None => match host.split_once(':') {
            Some((n, p)) => (n.to_string(), Some(p)),
            None => (host.clone(), None),
        },
    };
    // Anything after a bracketed literal that is not ":port" is malformed.
    if host.starts_with('[') && !host.ends_with(']') && port_part.is_none() {
        return false;
    }
    if !matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return false;
    }
    match port_part {
        None => true,
        Some(p) => p.parse::<u16>().is_ok_and(|p| p == port),
    }
}

///
/// Spawns an Axum server that listens on 127.0.0.1:{port} and shuts down
/// gracefully when the `shutdown` token is cancelled.
pub async fn start_web_server(
    port: u16,
    state: Arc<WebState>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let listener = bind_web_listener(port).await?;
    serve_web(listener, state, shutdown).await
}

/// Bind the web UI listener on 127.0.0.1:{port}.
///
/// Split from [`serve_web`] so the daemon can report a bind failure to its
/// parent instead of discovering it later inside a background task.
pub async fn bind_web_listener(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to bind web UI on port {port}: {e}. \
             Set [hub] web_port = <other> in mcp-hub.toml"
        )
    })?;
    tracing::info!("Web UI listening on http://{addr}");
    Ok(listener)
}

/// Serve the web UI on an already-bound `listener` until `shutdown` is cancelled.
pub async fn serve_web(
    listener: tokio::net::TcpListener,
    state: Arc<WebState>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::host_allowed;

    #[test]
    fn host_allowed_parses_forms() {
        assert!(host_allowed("localhost", 80));
        assert!(host_allowed("[::1]:80", 80));
        assert!(!host_allowed("[::1]x", 80));
        assert!(!host_allowed("[::1", 80));
        assert!(!host_allowed("localhost:80:80", 80));
        assert!(!host_allowed("localhost:+80", 81));
    }
}
