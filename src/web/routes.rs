#![allow(dead_code)]

use std::sync::Arc;

use askama::Template;
use askama_web::WebTemplate;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use super::WebState;
use crate::types::{format_uptime, HealthStatus, ProcessState};

// ─────────────────────────────────────────────────────────────────────────────
// Data structs used by templates
// ─────────────────────────────────────────────────────────────────────────────

/// Data for a single server card on the status page (D-05).
pub struct ServerCardData {
    pub name: String,
    /// CSS class for status dot: "running", "stopped", "fatal", "starting", "backoff", "stopping"
    pub state_class: String,
    /// Human-readable state: "running", "stopped", etc.
    pub state: String,
    /// CSS class for health badge: "healthy", "degraded", "failed", "unknown"
    pub health_class: String,
    /// Human-readable health: "healthy", "degraded (3 missed)", etc.
    pub health: String,
    /// PID as string, or "-" if not running
    pub pid: String,
    /// Uptime formatted as HH:MM:SS, or "-" if not running
    pub uptime: String,
    /// Number of restarts
    pub restart_count: u32,
    /// Tool/resource/prompt summary: "3T/1R/0P" or "-" if not introspected
    pub tool_count: String,
}

impl ServerCardData {
    /// Build a `ServerCardData` from a server name and its snapshot.
    pub fn from_snapshot(name: String, snap: &crate::types::ServerSnapshot) -> Self {
        let state_class = match &snap.process_state {
            ProcessState::Stopped => "stopped",
            ProcessState::Starting => "starting",
            ProcessState::Running => "running",
            ProcessState::Backoff { .. } => "backoff",
            ProcessState::Fatal => "fatal",
            ProcessState::Stopping => "stopping",
        }
        .to_string();

        let state = snap.process_state.to_string();

        let health_class = match &snap.health {
            HealthStatus::Unknown => "unknown",
            HealthStatus::Healthy { .. } => "healthy",
            HealthStatus::Degraded { .. } => "degraded",
            HealthStatus::Failed { .. } => "failed",
        }
        .to_string();

        let health = snap.health.to_string();

        let pid = snap
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());

        let uptime = snap
            .uptime_since
            .map(|s| format_uptime(s.elapsed()))
            .unwrap_or_else(|| "-".to_string());

        let tool_count = if snap.capabilities.introspected_at.is_some() {
            let caps = &snap.capabilities;
            format!(
                "{}T/{}R/{}P",
                caps.tools.len(),
                caps.resources.len(),
                caps.prompts.len()
            )
        } else {
            "-".to_string()
        };

        Self {
            name,
            state_class,
            state,
            health_class,
            health,
            pid,
            uptime,
            restart_count: snap.restart_count,
            tool_count,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-server entry in the /health JSON response.
// ─────────────────────────────────────────────────────────────────────────────

/// Per-server entry in the /health JSON response (D-13).
#[derive(Serialize)]
pub struct ServerHealthEntry {
    pub name: String,
    pub process_state: String,
    pub health: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
}

/// Response body for GET /health (WEB-05).
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub servers: Vec<ServerHealthEntry>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Data structs for tools browser
// ─────────────────────────────────────────────────────────────────────────────

/// Summary data for the tools browser accordion per D-08.
pub struct ServerToolsData {
    pub name: String,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub has_been_introspected: bool,
}

/// Tool detail entry for accordion expansion (D-09: name + description only).
pub struct ToolDetail {
    pub name: String,
    pub description: String,
}

/// Resource detail entry for accordion expansion.
pub struct ResourceDetail {
    pub name: String,
    pub uri: String,
    pub description: String,
}

/// Prompt detail entry for accordion expansion.
pub struct PromptDetail {
    pub name: String,
    pub description: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Server filter pill for logs page
// ─────────────────────────────────────────────────────────────────────────────

/// A filter pill for the logs page server filter (D-11).
pub struct ServerFilterPill {
    pub name: String,
    /// Whether this pill is the currently active filter.
    pub active: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Askama template structs
// ─────────────────────────────────────────────────────────────────────────────

/// Template for the Status tab (/).
#[derive(Template, WebTemplate)]
#[template(path = "status.html")]
pub struct StatusPage {
    pub active_tab: String,
    pub servers: Vec<ServerCardData>,
    pub total_count: usize,
    pub healthy_count: usize,
}

/// Fragment-only template for HTMX polling (D-06).
/// Returns just the card grid, NOT the full page.
#[derive(Template, WebTemplate)]
#[template(path = "status_partial.html")]
pub struct StatusPartial {
    pub servers: Vec<ServerCardData>,
}

/// Template for the Tools tab (/tools).
#[derive(Template, WebTemplate)]
#[template(path = "tools.html")]
pub struct ToolsPage {
    pub active_tab: String,
    pub servers: Vec<ServerToolsData>,
    pub total_count: usize,
}

/// Fragment for lazy-loaded accordion content for a single server.
#[derive(Template, WebTemplate)]
#[template(path = "tools_detail.html")]
pub struct ToolDetailPartial {
    pub server_name: String,
    pub tools: Vec<ToolDetail>,
    pub resources: Vec<ResourceDetail>,
    pub prompts: Vec<PromptDetail>,
}

/// Template for the Logs tab (/logs).
#[derive(Template, WebTemplate)]
#[template(path = "logs.html")]
pub struct LogsPage {
    pub active_tab: String,
    pub servers: Vec<ServerFilterPill>,
    pub active_filter: Option<String>,
    /// Pre-computed SSE URL: "/logs/stream" or "/logs/stream?server=name".
    /// Used directly in template as `sse-connect="{{ sse_url }}"`.
    pub sse_url: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Route handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET / -- Status tab with server card grid (WEB-02).
pub async fn status_page(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let handles = state.handles.lock().await;
    let servers: Vec<ServerCardData> = handles
        .iter()
        .map(|h| {
            let snap = h.state_rx.borrow().clone();
            ServerCardData::from_snapshot(h.name.clone(), &snap)
        })
        .collect();

    let total_count = servers.len();
    let healthy_count = servers
        .iter()
        .filter(|s| s.health_class == "healthy")
        .count();

    StatusPage {
        active_tab: "status".to_string(),
        servers,
        total_count,
        healthy_count,
    }
}

/// GET /partials/status -- HTMX polling fragment for auto-refresh every 3s (D-06).
pub async fn status_partial(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let handles = state.handles.lock().await;
    let servers: Vec<ServerCardData> = handles
        .iter()
        .map(|h| {
            let snap = h.state_rx.borrow().clone();
            ServerCardData::from_snapshot(h.name.clone(), &snap)
        })
        .collect();

    StatusPartial { servers }
}

/// GET /tools -- Tools accordion page (WEB-03, D-08).
pub async fn tools_page(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let handles = state.handles.lock().await;
    let servers: Vec<ServerToolsData> = handles
        .iter()
        .map(|h| {
            let snap = h.state_rx.borrow().clone();
            let caps = &snap.capabilities;
            ServerToolsData {
                name: h.name.clone(),
                tool_count: caps.tools.len(),
                resource_count: caps.resources.len(),
                prompt_count: caps.prompts.len(),
                has_been_introspected: caps.introspected_at.is_some(),
            }
        })
        .collect();

    let total_count = servers.len();

    ToolsPage {
        active_tab: "tools".to_string(),
        servers,
        total_count,
    }
}

/// GET /partials/tools/{server_name} -- Lazy-loaded accordion content for one server (D-09).
pub async fn tool_detail_partial(
    Path(server_name): Path<String>,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    let handles = state.handles.lock().await;
    let handle = handles.iter().find(|h| h.name == server_name);

    match handle {
        Some(h) => {
            let snap = h.state_rx.borrow().clone();
            let caps = &snap.capabilities;

            let tools: Vec<ToolDetail> = caps
                .tools
                .iter()
                .map(|t| ToolDetail {
                    name: t.name.clone(),
                    description: t.description.clone().unwrap_or_default(),
                })
                .collect();

            let resources: Vec<ResourceDetail> = caps
                .resources
                .iter()
                .map(|r| ResourceDetail {
                    name: r.name.clone(),
                    uri: r.uri.clone(),
                    description: r.description.clone().unwrap_or_default(),
                })
                .collect();

            let prompts: Vec<PromptDetail> = caps
                .prompts
                .iter()
                .map(|p| PromptDetail {
                    name: p.name.clone(),
                    description: p.description.clone().unwrap_or_default(),
                })
                .collect();

            ToolDetailPartial {
                server_name,
                tools,
                resources,
                prompts,
            }
            .into_response()
        }
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /logs -- Log streaming page (WEB-04).
pub async fn logs_page(
    State(state): State<Arc<WebState>>,
    Query(params): Query<super::sse::LogParams>,
) -> impl IntoResponse {
    let handles = state.handles.lock().await;
    let servers: Vec<ServerFilterPill> = handles
        .iter()
        .map(|h| ServerFilterPill {
            name: h.name.clone(),
            active: params.server.as_ref().is_some_and(|s| s == &h.name),
        })
        .collect();

    let sse_url = match &params.server {
        Some(name) => format!("/logs/stream?server={name}"),
        None => "/logs/stream".to_string(),
    };

    LogsPage {
        active_tab: "logs".to_string(),
        servers,
        active_filter: params.server.clone(),
        sse_url,
    }
}

/// GET /health -- JSON health endpoint for monitoring / load balancers (WEB-05, D-13).
///
/// Uses non-blocking `state_rx.borrow()` -- O(1) per server, responds in microseconds.
pub async fn health_handler(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let handles = state.handles.lock().await;

    let servers: Vec<ServerHealthEntry> = handles
        .iter()
        .map(|h| {
            let snap = h.state_rx.borrow().clone();
            ServerHealthEntry {
                name: h.name.clone(),
                process_state: snap.process_state.to_string(),
                health: snap.health.to_string(),
                pid: snap.pid,
                restart_count: snap.restart_count,
            }
        })
        .collect();

    let overall_status = if servers.is_empty() {
        "healthy"
    } else if servers.iter().any(|s| s.health.starts_with("failed")) {
        "failed"
    } else if servers.iter().all(|s| s.health == "healthy") {
        "healthy"
    } else {
        "degraded"
    };

    Json(HealthResponse {
        status: overall_status.to_string(),
        servers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn make_test_state() -> Arc<super::super::WebState> {
        let log_agg = Arc::new(crate::logs::LogAggregator::new(
            &["test-server".to_string()],
            100,
        ));
        let handles = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        Arc::new(super::super::WebState { handles, log_agg })
    }

    #[tokio::test]
    async fn status_page_returns_200() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("mcp-hub"));
    }

    #[tokio::test]
    async fn status_partial_returns_fragment() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/partials/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(!html.contains("<!DOCTYPE"));
    }

    #[tokio::test]
    async fn tools_page_returns_200() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn log_stream_returns_sse_content_type() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/logs/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("text/event-stream"));
    }

    #[tokio::test]
    async fn health_returns_json_with_status_and_servers() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("application/json"));
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("status").is_some());
        assert!(json.get("servers").is_some());
    }

    #[tokio::test]
    async fn health_responds_under_100ms() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let start = std::time::Instant::now();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(elapsed.as_millis() < 100);
    }

    #[tokio::test]
    async fn static_htmx_returns_javascript() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/static/htmx.min.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("javascript"));
    }

    #[tokio::test]
    async fn static_css_returns_stylesheet() {
        let state = make_test_state().await;
        let app = super::super::build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/static/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("text/css"));
    }
}
