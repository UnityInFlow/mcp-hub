#![allow(dead_code)]

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use super::WebState;

/// Query parameters for the SSE log stream endpoint.
#[derive(Deserialize)]
pub struct LogParams {
    /// Optional server name to filter logs. If absent, all servers are included.
    pub server: Option<String>,
}

/// GET /logs/stream -- Server-Sent Events stream of log lines.
///
/// Stub implementation — full SSE streaming logic is implemented in Plan 02.
/// Returns an empty response so the project compiles.
pub async fn log_stream_handler(
    Query(_params): Query<LogParams>,
    State(_state): State<Arc<WebState>>,
) -> impl IntoResponse {
    // Plan 02 will return a proper SSE stream via axum::response::sse::Sse.
    ""
}
