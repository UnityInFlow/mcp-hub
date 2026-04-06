#![allow(dead_code)]

use std::sync::Arc;

use askama::Template;
use askama_web::WebTemplate;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use super::WebState;

// ─────────────────────────────────────────────────────────────────────────────
// Data structs used by templates and API responses
// ─────────────────────────────────────────────────────────────────────────────

/// Summary data for a single server card on the Status page.
pub struct ServerCardData {
    pub name: String,
    pub process_state: String,
    pub health: String,
    pub pid: Option<u32>,
    pub uptime: String,
    pub restart_count: u32,
    pub transport: String,
    pub tool_count: usize,
}

/// Per-server entry in the /health JSON response.
#[derive(Serialize)]
pub struct ServerHealthEntry {
    pub name: String,
    pub state: String,
    pub health: String,
}

/// Response body for GET /health.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub servers: Vec<ServerHealthEntry>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Askama template structs
// ─────────────────────────────────────────────────────────────────────────────

/// Template for the Status tab (/).
#[derive(Template, WebTemplate)]
#[template(path = "status.html")]
pub struct StatusPage {
    pub active_tab: &'static str,
    pub total_count: usize,
    pub healthy_count: usize,
    pub servers: Vec<ServerCardData>,
}

/// Template for the Tools tab (/tools).
#[derive(Template, WebTemplate)]
#[template(path = "tools.html")]
pub struct ToolsPage {
    pub active_tab: &'static str,
    pub total_count: usize,
    pub servers: Vec<ServerCardData>,
}

/// Template for the Logs tab (/logs).
#[derive(Template, WebTemplate)]
#[template(path = "logs.html")]
pub struct LogsPage {
    pub active_tab: &'static str,
}

// ─────────────────────────────────────────────────────────────────────────────
// Route handlers (stubs -- full implementation in Plan 02)
// ─────────────────────────────────────────────────────────────────────────────

/// GET / -- Status tab with server card grid.
pub async fn status_page(State(_state): State<Arc<WebState>>) -> impl IntoResponse {
    StatusPage {
        active_tab: "status",
        total_count: 0,
        healthy_count: 0,
        servers: Vec::new(),
    }
}

/// GET /partials/status -- HTMX polling fragment for auto-refresh every 3s.
pub async fn status_partial(State(_state): State<Arc<WebState>>) -> impl IntoResponse {
    ""
}

/// GET /tools -- Tools accordion page.
pub async fn tools_page(State(_state): State<Arc<WebState>>) -> impl IntoResponse {
    ToolsPage {
        active_tab: "tools",
        total_count: 0,
        servers: Vec::new(),
    }
}

/// GET /partials/tools/{server_name} -- Lazy-loaded accordion content for one server.
pub async fn tool_detail_partial(
    Path(_server_name): Path<String>,
    State(_state): State<Arc<WebState>>,
) -> impl IntoResponse {
    ""
}

/// GET /logs -- Log streaming page.
pub async fn logs_page(State(_state): State<Arc<WebState>>) -> impl IntoResponse {
    LogsPage { active_tab: "logs" }
}

/// GET /health -- JSON health endpoint for monitoring / load balancers.
pub async fn health_handler(State(_state): State<Arc<WebState>>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        servers: Vec::new(),
    })
}
