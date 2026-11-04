//! `transport = "http"` servers (issue #59): the process is supervised, but the
//! stdio JSON-RPC health loop and introspection must not run against it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use mcp_hub::config::ServerConfig;
use mcp_hub::logs::LogAggregator;
use mcp_hub::output::format_status_table;
use mcp_hub::supervisor::{start_single_server, ServerHandle};
use mcp_hub::types::{HealthStatus, ProcessState, ServerSnapshot};
use mcp_hub::web::{build_router, new_server_views, WebState};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;

fn config(transport: &str, command: &str, args: &[&str]) -> ServerConfig {
    ServerConfig {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
        env_file: None,
        transport: transport.to_string(),
        cwd: None,
        health_check_interval: Some(1),
        max_retries: None,
        restart_delay: None,
    }
}

/// Stop the supervised process (and its group) so no `sleep` outlives a test.
async fn stop(handle: ServerHandle, shutdown: &CancellationToken) {
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), handle.task).await;
}

/// Wait until `pred` holds for the snapshot, up to `max`. True if it held.
async fn wait_for(
    handle: &ServerHandle,
    max: Duration,
    pred: impl Fn(&ServerSnapshot) -> bool,
) -> bool {
    let mut rx = handle.state_rx.clone();
    tokio::time::timeout(max, async {
        loop {
            if pred(&rx.borrow_and_update()) {
                return;
            }
            if rx.changed().await.is_err() {
                // Sender gone: the supervisor exited; the state can no longer change.
                std::future::pending::<()>().await;
            }
        }
    })
    .await
    .is_ok()
}

/// 60 s of virtual time is well past 7 pings at a 1 s interval (+5 s timeouts).
const LONG: Duration = Duration::from_secs(60);

#[tokio::test(start_paused = true)]
async fn http_server_stays_running_and_not_checked_past_failure_threshold() {
    let shutdown = CancellationToken::new();
    let agg = Arc::new(LogAggregator::new(&[], 100));
    let handle =
        start_single_server("web", &config("http", "sleep", &["600"]), &shutdown, &agg).await;

    // Never reaches Degraded/Failed no matter how long we wait.
    let went_bad = wait_for(&handle, LONG, |s| {
        matches!(
            s.health,
            HealthStatus::Degraded { .. } | HealthStatus::Failed { .. }
        )
    })
    .await;
    let snap = handle.state_rx.borrow().clone();
    stop(handle, &shutdown).await;

    assert!(!went_bad, "http server must never degrade: {snap:?}");
    assert!(matches!(snap.process_state, ProcessState::Running));
    assert!(
        matches!(snap.health, HealthStatus::NotChecked),
        "got {:?}",
        snap.health
    );
    assert_eq!(snap.health.to_string(), "not checked");
    assert_eq!(snap.transport, "http");
    assert!(snap.capabilities.introspected_at.is_none());
}

/// Regression guard: a stdio server that never answers still goes Failed.
#[tokio::test(start_paused = true)]
async fn silent_stdio_server_still_goes_failed() {
    let shutdown = CancellationToken::new();
    let agg = Arc::new(LogAggregator::new(&[], 100));
    let handle =
        start_single_server("mute", &config("stdio", "sleep", &["600"]), &shutdown, &agg).await;

    let failed = wait_for(&handle, Duration::from_secs(120), |s| {
        matches!(s.health, HealthStatus::Failed { .. })
    })
    .await;
    stop(handle, &shutdown).await;
    assert!(failed, "unresponsive stdio server must reach Failed");
}

#[tokio::test]
async fn http_server_stdout_lines_reach_the_log_buffer() {
    let shutdown = CancellationToken::new();
    let agg = Arc::new(LogAggregator::new(&[], 100));
    let handle = start_single_server(
        "chatty",
        &config(
            "http",
            "sh",
            &["-c", "echo out-line-1; echo err-line-1 >&2; exec sleep 600"],
        ),
        &shutdown,
        &agg,
    )
    .await;

    let buffer = agg.ensure_buffer("chatty");
    let mut messages = Vec::new();
    for _ in 0..100 {
        messages = buffer
            .snapshot()
            .await
            .into_iter()
            .map(|l| l.message)
            .collect();
        if messages.iter().any(|m| m == "out-line-1") && messages.iter().any(|m| m == "err-line-1")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop(handle, &shutdown).await;
    assert!(messages.contains(&"out-line-1".to_string()), "{messages:?}");
    assert!(messages.contains(&"err-line-1".to_string()), "{messages:?}");
}

/// stdin is /dev/null for http servers: `cat` sees EOF at once instead of
/// blocking on an open-but-unused pipe.
#[tokio::test]
async fn http_server_stdin_is_closed() {
    let shutdown = CancellationToken::new();
    let agg = Arc::new(LogAggregator::new(&[], 100));
    let cfg = config("http", "sh", &["-c", "cat; echo stdin-eof; exec sleep 600"]);
    let handle = start_single_server("eof", &cfg, &shutdown, &agg).await;

    let buffer = agg.ensure_buffer("eof");
    let mut seen = false;
    for _ in 0..100 {
        seen = buffer
            .snapshot()
            .await
            .iter()
            .any(|l| l.message == "stdin-eof");
        if seen {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop(handle, &shutdown).await;
    assert!(seen, "stdin should be at EOF for an http server");
}

// ── Status table ────────────────────────────────────────────────────────────

#[test]
fn status_table_shows_not_checked_for_http_server() {
    let snap = ServerSnapshot {
        process_state: ProcessState::Running,
        health: HealthStatus::NotChecked,
        pid: Some(4242),
        transport: "http".to_string(),
        ..ServerSnapshot::default()
    };
    let out = format_status_table(&[("web".to_string(), snap)], false);
    assert!(out.contains("not checked"), "{out}");
    assert!(out.contains("http"), "{out}");
}

// ── /health ─────────────────────────────────────────────────────────────────

fn healthy() -> HealthStatus {
    HealthStatus::Healthy {
        latency_ms: 3,
        last_checked: std::time::Instant::now(),
    }
}

async fn health(servers: Vec<(&str, HealthStatus)>) -> (StatusCode, serde_json::Value) {
    let names: Vec<String> = servers.iter().map(|(n, _)| (*n).to_string()).collect();
    let log_agg = Arc::new(LogAggregator::new(&names, 100));
    let mut handles = Vec::new();
    let mut senders = Vec::new();
    for (name, health) in servers {
        let snapshot = ServerSnapshot {
            process_state: ProcessState::Running,
            health,
            ..ServerSnapshot::default()
        };
        let (tx, state_rx) = tokio::sync::watch::channel(snapshot);
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
        handles.push(ServerHandle {
            name: name.to_string(),
            state_rx,
            cmd_tx,
            task: tokio::spawn(async {}),
        });
        senders.push(tx);
    }
    let state = Arc::new(WebState {
        servers: new_server_views(&handles),
        log_agg,
        port: 3456,
    });
    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let code = resp.status();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (code, serde_json::from_slice(&body).expect("json"))
}

#[tokio::test]
async fn health_is_200_healthy_with_stdio_healthy_and_http_not_checked() {
    let (code, json) = health(vec![("a", healthy()), ("web", HealthStatus::NotChecked)]).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(json["status"], "healthy");
    assert_eq!(json["servers"][1]["health"], "not checked");
}

#[tokio::test]
async fn health_is_503_with_failed_stdio_even_alongside_not_checked() {
    let (code, json) = health(vec![
        ("web", HealthStatus::NotChecked),
        (
            "bad",
            HealthStatus::Failed {
                consecutive_misses: 7,
            },
        ),
    ])
    .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["status"], "failed");
}
