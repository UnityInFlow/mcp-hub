//! Dispatcher unit tests — covers reader_task routing, send_request timeout,
//! pending map drain on stdout close, and concurrent request resolution.

use std::collections::HashMap;
use std::sync::Arc;

use mcp_hub::mcp::dispatcher::{reader_task, send_request, IdAllocator, PendingMap, SharedStdin};
use tokio::sync::Mutex;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build an empty PendingMap.
fn make_pending() -> PendingMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Bash echo server: reads lines and echoes `{"jsonrpc":"2.0","result":{},"id":<id>}`.
#[cfg(unix)]
const ECHO_SCRIPT: &str = r#"
while IFS= read -r line; do
    id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
    echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
done
"#;

/// Spawn an echo responder and wire up the dispatcher.
/// Returns (child, SharedStdin, PendingMap).
#[cfg(unix)]
async fn spawn_echo_dispatcher() -> (tokio::process::Child, SharedStdin, PendingMap) {
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(ECHO_SCRIPT)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
    let pending = make_pending();

    let pending_clone = Arc::clone(&pending);
    tokio::spawn(async move {
        reader_task(stdout, pending_clone).await;
    });

    (child, stdin_shared, pending)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn dispatcher_send_request_returns_correct_response() {
    use mcp_hub::mcp::protocol::PingRequest;

    let (mut child, stdin_shared, pending) = spawn_echo_dispatcher().await;

    let request = PingRequest::new(7);
    let response = send_request(&stdin_shared, &pending, 7, &request, 5).await;

    child.kill().await.ok();

    let response = response.expect("send_request should succeed");
    assert_eq!(response.id, 7, "response id should match request id");
    assert!(response.error.is_none(), "response should not have error");
    assert!(response.result.is_some(), "response should have result");
}

#[cfg(unix)]
#[tokio::test]
async fn dispatcher_send_request_times_out_when_no_response() {
    // A server that reads but never responds.
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg("cat > /dev/null")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
    let pending = make_pending();

    let pending_clone = Arc::clone(&pending);
    tokio::spawn(async move {
        reader_task(stdout, pending_clone).await;
    });

    use mcp_hub::mcp::protocol::PingRequest;
    let request = PingRequest::new(42);

    let start = std::time::Instant::now();
    // Use a 1-second timeout so the test runs fast.
    let result = send_request(&stdin_shared, &pending, 42, &request, 1).await;
    let elapsed = start.elapsed();

    child.kill().await.ok();

    assert!(result.is_err(), "send_request should time out");
    assert!(
        elapsed.as_secs() < 3,
        "timeout should complete within 3 seconds, took {}s",
        elapsed.as_secs()
    );

    // Pending map should be clean after timeout (no leaked entry).
    let map = pending.lock().await;
    assert!(
        map.is_empty(),
        "pending map should be empty after timeout cleanup"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn reader_task_drains_pending_map_on_stdout_close() {
    // A server that exits immediately without responding.
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg("exit 0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("bash should be available");

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let _stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
    let pending = make_pending();

    // Pre-register a pending waiter before spawning reader_task.
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut map = pending.lock().await;
        // Use a fake JsonRpcResponse to satisfy the type — insert a fake waiter.
        map.insert(99, tx);
    }

    let pending_clone = Arc::clone(&pending);
    let reader_handle = tokio::spawn(async move {
        reader_task(stdout, pending_clone).await;
    });

    // reader_task should drain the pending map when stdout closes.
    // The pre-registered receiver should get RecvError.
    let recv_result = tokio::time::timeout(std::time::Duration::from_secs(3), rx).await;

    reader_handle.await.ok();
    child.wait().await.ok();

    // The receiver should get an error (sender was dropped by drain).
    assert!(
        recv_result.is_ok(),
        "receiver future should have resolved (not timed out)"
    );
    assert!(
        recv_result.unwrap().is_err(),
        "receiver should get RecvError when stdout closes and map is drained"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn dispatcher_concurrent_requests() {
    use mcp_hub::mcp::protocol::PingRequest;

    let (mut child, stdin_shared, pending) = spawn_echo_dispatcher().await;

    // Send 5 concurrent requests with different IDs.
    let mut handles = Vec::new();
    for i in 1u64..=5 {
        let stdin_clone = Arc::clone(&stdin_shared);
        let pending_clone = Arc::clone(&pending);
        handles.push(tokio::spawn(async move {
            let request = PingRequest::new(i);
            send_request(&stdin_clone, &pending_clone, i, &request, 5).await
        }));
    }

    let mut responses: Vec<u64> = Vec::new();
    for handle in handles {
        let result = handle.await.expect("task should not panic");
        let response = result.expect("send_request should succeed");
        responses.push(response.id);
    }

    child.kill().await.ok();

    // All 5 IDs should be present (possibly out of order).
    responses.sort();
    assert_eq!(
        responses,
        vec![1, 2, 3, 4, 5],
        "all 5 concurrent requests should resolve with correct IDs"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// IdAllocator unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn id_allocator_starts_at_one() {
    let alloc = IdAllocator::new();
    assert_eq!(alloc.next_id(), 1, "first ID should be 1");
}

#[test]
fn id_allocator_increments_monotonically() {
    let alloc = IdAllocator::new();
    let ids: Vec<u64> = (0..5).map(|_| alloc.next_id()).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "IDs should increment by 1");
}

#[test]
fn id_allocator_default_matches_new() {
    let alloc = IdAllocator::default();
    assert_eq!(alloc.next_id(), 1, "default() should start at 1");
}

// ─────────────────────────────────────────────────────────────────────────────
// Non-unix stubs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(unix))]
#[test]
fn dispatcher_tests_require_unix() {
    // bash-based integration tests are Unix-only.
}
