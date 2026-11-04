//! Web UI route tests: SSE `?server=` filtering, `/partials/tools/{name}`
//! status codes, and HTML-escaping regression guards for server names and
//! tool/resource/prompt descriptions.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use mcp_hub::logs::LogAggregator;
use mcp_hub::mcp::protocol::{McpPrompt, McpResource, McpTool};
use mcp_hub::supervisor::ServerHandle;
use mcp_hub::types::{McpCapabilities, ServerSnapshot};
use mcp_hub::web::{build_router, new_server_views, WebState};
use tower::ServiceExt as _;

/// Test fixture: web state plus the senders that keep the snapshots alive.
struct Fixture {
    state: Arc<WebState>,
    _senders: Vec<tokio::sync::watch::Sender<ServerSnapshot>>,
}

/// Build a web state with one server per `(name, capabilities)` pair.
fn fixture(servers: Vec<(&str, McpCapabilities)>) -> Fixture {
    let names: Vec<String> = servers.iter().map(|(n, _)| (*n).to_string()).collect();
    let log_agg = Arc::new(LogAggregator::new(&names, 100));
    let mut handles = Vec::new();
    let mut senders = Vec::new();
    for (name, capabilities) in servers {
        let snapshot = ServerSnapshot {
            capabilities,
            ..ServerSnapshot::default()
        };
        let (state_tx, state_rx) = tokio::sync::watch::channel(snapshot);
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
        handles.push(ServerHandle {
            name: name.to_string(),
            state_rx,
            cmd_tx,
            task: tokio::spawn(async {}),
        });
        senders.push(state_tx);
    }
    let servers = new_server_views(&handles);
    Fixture {
        state: Arc::new(WebState { servers, log_agg }),
        _senders: senders,
    }
}

async fn get(state: &Arc<WebState>, uri: &str) -> (StatusCode, String) {
    let resp = build_router(Arc::clone(state))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn caps(tool_desc: &str) -> McpCapabilities {
    McpCapabilities {
        tools: vec![McpTool {
            name: "do-thing".to_string(),
            description: Some(tool_desc.to_string()),
            input_schema: None,
        }],
        resources: vec![McpResource {
            uri: "file:///x".to_string(),
            name: "res".to_string(),
            description: Some(tool_desc.to_string()),
            mime_type: None,
        }],
        prompts: vec![McpPrompt {
            name: "prompt".to_string(),
            description: Some(tool_desc.to_string()),
            arguments: None,
        }],
        introspected_at: Some(std::time::Instant::now()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SSE filtering
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn log_stream_with_server_filter_only_yields_that_server() {
    let fx = fixture(vec![
        ("a", McpCapabilities::default()),
        ("b", McpCapabilities::default()),
    ]);
    let resp = build_router(Arc::clone(&fx.state))
        .oneshot(
            Request::builder()
                .uri("/logs/stream?server=a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    // The handler has subscribed by the time the response exists.
    fx.state.log_agg.push("a", "first-from-a".to_string()).await;
    fx.state.log_agg.push("b", "only-from-b".to_string()).await;
    fx.state
        .log_agg
        .push("a", "second-from-a".to_string())
        .await;

    // Read frames until the last `a` line arrives (or give up).
    let mut received = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while !received.contains("second-from-a") {
            match body.frame().await {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        received.push_str(&String::from_utf8_lossy(data));
                    }
                }
                Some(Err(_)) | None => return,
            }
        }
    })
    .await;

    assert!(received.contains("first-from-a"), "got: {received}");
    assert!(received.contains("second-from-a"), "got: {received}");
    assert!(
        !received.contains("only-from-b"),
        "server=a stream leaked a line from b: {received}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// /partials/tools/{name}
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tools_partial_known_server_is_200_and_unknown_is_404() {
    let fx = fixture(vec![("known", caps("does a thing"))]);

    let (status, html) = get(&fx.state, "/partials/tools/known").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("do-thing"), "got: {html}");
    assert!(html.contains("does a thing"), "got: {html}");

    let (status, _) = get(&fx.state, "/partials/tools/unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ─────────────────────────────────────────────────────────────────────────────
// HTML escaping (regression guards — Askama escapes by default)
// ─────────────────────────────────────────────────────────────────────────────

const EVIL_NAME: &str = "<script>alert(1)</script>";

/// The payload must appear escaped, never as live markup.
fn assert_escaped(html: &str, payload_text: &str, raw_markup: &str) {
    assert!(
        !html.contains(raw_markup),
        "raw markup {raw_markup:?} found unescaped in: {html}"
    );
    assert!(
        html.contains(payload_text),
        "escaped payload text {payload_text:?} missing from: {html}"
    );
}

#[tokio::test]
async fn server_name_is_escaped_on_status_and_tools_pages() {
    let fx = fixture(vec![(EVIL_NAME, McpCapabilities::default())]);
    for uri in ["/", "/partials/status", "/tools"] {
        let (status, html) = get(&fx.state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_escaped(&html, "alert(1)", "<script>alert(1)");
    }
}

#[tokio::test]
async fn tool_descriptions_are_escaped_in_tools_partial() {
    let evil = "<script>alert(2)</script>";
    let fx = fixture(vec![("srv", caps(evil))]);
    let (status, html) = get(&fx.state, "/partials/tools/srv").await;
    assert_eq!(status, StatusCode::OK);
    assert_escaped(&html, "alert(2)", "<script>alert(2)");
}
