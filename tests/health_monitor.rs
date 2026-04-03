//! Health monitor tests — covers PingRequest serialization, JsonRpcResponse deserialization,
//! and the ping_server / run_health_check_loop integration with a mock MCP echo server.

use mcp_hub::mcp::health::{run_health_check_loop, DEFAULT_HEALTH_CHECK_INTERVAL_SECS};
use mcp_hub::mcp::protocol::{JsonRpcResponse, PingRequest};
use mcp_hub::types::{HealthStatus, ServerSnapshot};
use tokio_util::sync::CancellationToken;

// ─────────────────────────────────────────────────────────────────────────────
// PingRequest serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ping_request_serialization() {
    let req = PingRequest::new(42);
    let json = serde_json::to_string(&req).expect("serialization should not fail");

    assert!(
        json.contains(r#""jsonrpc":"2.0""#),
        "missing jsonrpc field: {json}"
    );
    assert!(
        json.contains(r#""method":"ping""#),
        "missing method field: {json}"
    );
    assert!(json.contains(r#""id":42"#), "missing id field: {json}");
}

#[test]
fn ping_request_id_zero() {
    let req = PingRequest::new(0);
    let json = serde_json::to_string(&req).expect("serialization should not fail");
    assert!(json.contains(r#""id":0"#), "id should be 0: {json}");
}

#[test]
fn ping_request_large_id() {
    let req = PingRequest::new(u64::MAX);
    let json = serde_json::to_string(&req).expect("serialization should not fail");
    assert!(
        json.contains(&u64::MAX.to_string()),
        "large id should be present: {json}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// JsonRpcResponse deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn successful_ping_response() {
    let json = r#"{"jsonrpc":"2.0","result":{},"id":1}"#;
    let resp: JsonRpcResponse = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(resp.id, 1, "id should be 1");
    assert!(resp.result.is_some(), "result should be Some");
    assert!(resp.error.is_none(), "error should be None");
}

#[test]
fn error_ping_response() {
    let json = r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":1}"#;
    let resp: JsonRpcResponse = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(resp.id, 1, "id should be 1");
    assert!(resp.error.is_some(), "error should be Some");
}

#[test]
fn ping_response_missing_result_field() {
    // Some servers only send error or only send result — missing field should be fine.
    let json = r#"{"jsonrpc":"2.0","id":5}"#;
    let resp: JsonRpcResponse =
        serde_json::from_str(json).expect("should deserialize with missing fields");

    assert_eq!(resp.id, 5);
    assert!(resp.result.is_none());
    assert!(resp.error.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// DEFAULT_HEALTH_CHECK_INTERVAL_SECS constant
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn default_health_check_interval_is_reasonable() {
    // Must be positive and not absurdly large (sanity check).
    assert!(DEFAULT_HEALTH_CHECK_INTERVAL_SECS > 0);
    assert!(DEFAULT_HEALTH_CHECK_INTERVAL_SECS <= 300);
}

// ─────────────────────────────────────────────────────────────────────────────
// Ping integration test — mock MCP echo server via bash
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal bash script that reads JSON-RPC pings on stdin and echoes responses on stdout.
///
/// For each line read, it extracts the `id` field and replies with
/// `{"jsonrpc":"2.0","result":{},"id":<id>}`.
#[cfg(unix)]
const PING_RESPONDER_SCRIPT: &str = r#"
while IFS= read -r line; do
    id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
    echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
done
"#;

#[cfg(unix)]
#[tokio::test]
async fn ping_server_returns_latency_for_valid_responder() {
    use mcp_hub::mcp::health::ping_server;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(PING_RESPONDER_SCRIPT)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("bash should be available");

    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");
    let mut lines = BufReader::new(stdout).lines();

    let result = ping_server(&mut stdin, &mut lines, 1).await;

    child.kill().await.ok();

    assert!(
        result.is_ok(),
        "ping_server should succeed with a responding server, got: {:?}",
        result
    );

    let latency = result.unwrap();
    // Latency should be a small positive number (bash start + echo is < 5000ms).
    assert!(
        latency < 5000,
        "latency should be under 5 seconds: {latency}ms"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ping_timeout() {
    use mcp_hub::mcp::health::ping_server;
    use tokio::io::{AsyncBufReadExt, BufReader};

    // A process that reads stdin but never writes to stdout.
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg("cat > /dev/null")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");
    let mut lines = BufReader::new(stdout).lines();

    let start = std::time::Instant::now();
    let result = ping_server(&mut stdin, &mut lines, 99).await;
    let elapsed = start.elapsed();

    child.kill().await.ok();

    assert!(
        result.is_err(),
        "ping_server should time out when server does not respond"
    );

    // Should have timed out within ~6 seconds (5s timeout + some overhead).
    assert!(
        elapsed.as_secs() < 8,
        "timeout should complete within 8 seconds, took {}s",
        elapsed.as_secs()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Health check loop integration — verifies watch channel gets Healthy
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn run_health_check_loop_sets_healthy_on_first_ping() {
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(PING_RESPONDER_SCRIPT)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");

    let initial_snapshot = ServerSnapshot::default();
    let (tx, rx) = tokio::sync::watch::channel(initial_snapshot);

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Use a very short interval (100ms) so the test completes quickly.
    let handle = tokio::spawn(async move {
        run_health_check_loop(
            "test-server".to_string(),
            // interval_secs must be u64, use 0 would panic — minimum is 1 for interval.
            // We use a very short delay by passing 1 second and just waiting.
            1,
            stdin,
            stdout,
            tx,
            cancel_clone,
        )
        .await;
    });

    // Wait up to 3 seconds for the health to become Healthy.
    let became_healthy = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut rx_clone = rx.clone();
        loop {
            let snapshot = rx_clone.borrow().clone();
            if matches!(snapshot.health, HealthStatus::Healthy { .. }) {
                return true;
            }
            if rx_clone.changed().await.is_err() {
                return false;
            }
        }
    })
    .await;

    // Cancel and clean up.
    cancel.cancel();
    handle.await.ok();
    child.kill().await.ok();

    assert!(
        became_healthy.is_ok() && became_healthy.unwrap(),
        "health should become Healthy after a successful ping"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn run_health_check_loop_exits_on_cancel() {
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(PING_RESPONDER_SCRIPT)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");

    let initial_snapshot = ServerSnapshot::default();
    let (tx, _rx) = tokio::sync::watch::channel(initial_snapshot);

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let handle = tokio::spawn(async move {
        run_health_check_loop(
            "test-cancel".to_string(),
            60, // long interval — we test cancellation before first tick
            stdin,
            stdout,
            tx,
            cancel_clone,
        )
        .await;
    });

    // Cancel immediately.
    cancel.cancel();

    // The task should exit cleanly within 1 second.
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;

    child.kill().await.ok();

    assert!(
        result.is_ok(),
        "health check loop should exit cleanly after cancellation"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Non-unix stubs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(unix))]
#[test]
fn ping_server_tests_require_unix() {
    // bash-based integration tests are Unix-only.
    // On Windows, these would need a PowerShell or cmd equivalent.
    // For now, we skip them with a passing stub.
}
