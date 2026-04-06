#![allow(dead_code)]

use axum::http::header;
/// Embedded static assets -- per D-04, all assets compiled into the binary.
use axum::response::IntoResponse;

/// HTMX core library (~14KB gzipped).
pub const HTMX_JS: &str = include_str!("../../static/htmx.min.js");

/// HTMX SSE extension -- required for hx-ext="sse" (per D-10, Pitfall 7).
pub const HTMX_SSE_JS: &str = include_str!("../../static/htmx-sse.js");

/// Application CSS stylesheet.
pub const STYLE_CSS: &str = include_str!("../../static/style.css");

/// Serve the HTMX core library.
pub async fn serve_htmx() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], HTMX_JS)
}

/// Serve the HTMX SSE extension.
pub async fn serve_htmx_sse() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        HTMX_SSE_JS,
    )
}

/// Serve the application CSS stylesheet.
pub async fn serve_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}
