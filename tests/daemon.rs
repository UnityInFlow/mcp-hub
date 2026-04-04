/// Tests for daemon mode: PID file management, socket paths, IPC types,
/// and the control socket round-trip.
use std::sync::Arc;

use mcp_hub::control::{run_control_socket, DaemonRequest, DaemonResponse, DaemonState};
use mcp_hub::daemon::{pid_path, remove_pid_file, socket_path, write_pid_file};
use mcp_hub::logs::LogAggregator;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn socket_path_returns_valid() {
    let path = socket_path().expect("socket_path should succeed");
    assert!(
        path.to_string_lossy().ends_with("mcp-hub.sock"),
        "socket path should end with mcp-hub.sock, got: {path:?}"
    );
    // The parent directory must exist (socket_path creates it).
    assert!(
        path.parent().expect("socket path has a parent").exists(),
        "parent directory of socket path should exist"
    );
}

#[test]
fn pid_path_returns_valid() {
    let path = pid_path().expect("pid_path should succeed");
    assert!(
        path.to_string_lossy().ends_with("mcp-hub.pid"),
        "PID path should end with mcp-hub.pid, got: {path:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// PID file tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn write_and_read_pid_file() {
    let dir = TempDir::new().expect("temp dir");
    let pid_file = dir.path().join("test.pid");

    write_pid_file(&pid_file).expect("write_pid_file should succeed");

    let content = std::fs::read_to_string(&pid_file).expect("read PID file");
    let parsed: u32 = content
        .trim()
        .parse()
        .expect("PID file should contain a valid u32");

    assert_eq!(
        parsed,
        std::process::id(),
        "PID file should contain the current process ID"
    );
}

#[test]
fn remove_pid_file_deletes_file() {
    let dir = TempDir::new().expect("temp dir");
    let pid_file = dir.path().join("test.pid");

    write_pid_file(&pid_file).expect("write PID file");
    assert!(pid_file.exists(), "PID file should exist after write");

    remove_pid_file(&pid_file);
    assert!(!pid_file.exists(), "PID file should be removed");
}

#[test]
fn remove_pid_file_is_idempotent() {
    let dir = TempDir::new().expect("temp dir");
    let pid_file = dir.path().join("nonexistent.pid");

    // Should not panic or return an error even if the file does not exist.
    remove_pid_file(&pid_file);
}

// ─────────────────────────────────────────────────────────────────────────────
// Duplicate daemon check test (Unix only)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn check_existing_daemon_with_no_socket() {
    use mcp_hub::daemon::check_existing_daemon;

    let dir = TempDir::new().expect("temp dir");
    let sock = dir.path().join("nonexistent.sock");
    let pid = dir.path().join("nonexistent.pid");

    // No socket exists — check should succeed (no daemon running).
    let result = check_existing_daemon(&sock, &pid);
    assert!(result.is_ok(), "should return Ok when no socket exists");
}

// ─────────────────────────────────────────────────────────────────────────────
// DaemonRequest serialization round-trip
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn daemon_request_serialization() {
    // Status
    let status_json = serde_json::to_string(&DaemonRequest::Status).unwrap();
    assert_eq!(status_json, r#"{"cmd":"status"}"#);
    let back: DaemonRequest = serde_json::from_str(&status_json).unwrap();
    assert!(matches!(back, DaemonRequest::Status));

    // Stop
    let stop_json = serde_json::to_string(&DaemonRequest::Stop).unwrap();
    assert_eq!(stop_json, r#"{"cmd":"stop"}"#);
    let back: DaemonRequest = serde_json::from_str(&stop_json).unwrap();
    assert!(matches!(back, DaemonRequest::Stop));

    // Restart
    let restart_json = serde_json::to_string(&DaemonRequest::Restart {
        name: "foo".to_string(),
    })
    .unwrap();
    assert_eq!(restart_json, r#"{"cmd":"restart","name":"foo"}"#);
    let back: DaemonRequest = serde_json::from_str(&restart_json).unwrap();
    assert!(matches!(back, DaemonRequest::Restart { name } if name == "foo"));

    // Logs (with server)
    let logs_json = serde_json::to_string(&DaemonRequest::Logs {
        server: Some("bar".to_string()),
        lines: 50,
    })
    .unwrap();
    let back: DaemonRequest = serde_json::from_str(&logs_json).unwrap();
    assert!(matches!(back, DaemonRequest::Logs { server: Some(ref s), lines: 50 } if s == "bar"));

    // Logs (all servers)
    let logs_all_json = serde_json::to_string(&DaemonRequest::Logs {
        server: None,
        lines: 100,
    })
    .unwrap();
    let back: DaemonRequest = serde_json::from_str(&logs_all_json).unwrap();
    assert!(matches!(
        back,
        DaemonRequest::Logs {
            server: None,
            lines: 100
        }
    ));

    // Reload
    let reload_json = serde_json::to_string(&DaemonRequest::Reload).unwrap();
    assert_eq!(reload_json, r#"{"cmd":"reload"}"#);
    let back: DaemonRequest = serde_json::from_str(&reload_json).unwrap();
    assert!(matches!(back, DaemonRequest::Reload));
}

// ─────────────────────────────────────────────────────────────────────────────
// DaemonResponse constructor tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn daemon_response_constructors() {
    // success
    let resp = DaemonResponse::success(json!("test"));
    assert!(resp.ok, "success response should have ok=true");
    assert!(resp.data.is_some(), "success response should have data");
    assert!(
        resp.error.is_none(),
        "success response should not have error"
    );

    // ok_empty
    let resp = DaemonResponse::ok_empty();
    assert!(resp.ok, "ok_empty response should have ok=true");
    assert!(resp.data.is_none(), "ok_empty response should have no data");
    assert!(
        resp.error.is_none(),
        "ok_empty response should have no error"
    );

    // err
    let resp = DaemonResponse::err("fail".to_string());
    assert!(!resp.ok, "error response should have ok=false");
    assert!(resp.data.is_none(), "error response should have no data");
    assert_eq!(
        resp.error.as_deref(),
        Some("fail"),
        "error response should contain the error message"
    );
}

#[test]
fn daemon_response_serialization() {
    // success serialises data, omits error
    let resp = DaemonResponse::success(json!({"servers": []}));
    let json_str = serde_json::to_string(&resp).unwrap();
    assert!(json_str.contains(r#""ok":true"#));
    assert!(json_str.contains(r#""data""#));
    assert!(!json_str.contains(r#""error""#));

    // err omits data, includes error
    let resp = DaemonResponse::err("something went wrong".to_string());
    let json_str = serde_json::to_string(&resp).unwrap();
    assert!(json_str.contains(r#""ok":false"#));
    assert!(!json_str.contains(r#""data""#));
    assert!(json_str.contains(r#""error""#));
}

// ─────────────────────────────────────────────────────────────────────────────
// Control socket round-trip (integration)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn control_socket_round_trip() {
    use mcp_hub::control::send_daemon_command;

    let dir = TempDir::new().expect("temp dir");
    let sock_path = dir.path().join("test-control.sock");

    // Build a minimal DaemonState with no actual servers.
    let shutdown = CancellationToken::new();
    let log_agg = Arc::new(LogAggregator::new(&[], 1000));
    let state = Arc::new(DaemonState {
        handles: Arc::new(tokio::sync::Mutex::new(vec![])),
        log_agg: Arc::clone(&log_agg),
        shutdown: shutdown.clone(),
        color: false,
    });

    // Spawn the control socket listener.
    let sock_for_server = sock_path.clone();
    let server_handle = tokio::spawn(async move {
        run_control_socket(&sock_for_server, state)
            .await
            .expect("control socket should run without error");
    });

    // Give the listener a moment to bind.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send a Status request via the client helper.
    let response = send_daemon_command(&sock_path, &DaemonRequest::Status, 5)
        .await
        .expect("send_daemon_command should succeed");

    assert!(response.ok, "Status response should be ok=true");
    let data = response.data.expect("Status response should have data");
    assert!(data.is_array(), "Status data should be a JSON array");

    // Send a Stop request — this cancels the shutdown token.
    let stop_response = send_daemon_command(&sock_path, &DaemonRequest::Stop, 5)
        .await
        .expect("Stop command should succeed");

    assert!(stop_response.ok, "Stop response should be ok=true");

    // Wait for the control socket task to exit.
    server_handle.await.expect("server task should complete");
}
