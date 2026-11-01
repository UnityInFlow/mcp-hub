//! Dispatcher unit tests — covers reader_task routing, send_request timeout,
//! pending map drain on stdout close, and concurrent request resolution.

use std::collections::HashMap;
use std::sync::Arc;

use mcp_hub::mcp::dispatcher::{reader_task, send_request, IdAllocator, PendingMap, SharedStdin};
use mcp_hub::mcp::protocol::JsonRpcId;
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
    assert_eq!(
        response.id,
        JsonRpcId::Number(7),
        "response id should match request id"
    );
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
        match response.id {
            JsonRpcId::Number(n) => responses.push(n),
            JsonRpcId::String(s) => panic!("unexpected string id {s}"),
        }
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
// Strict response routing (issue #24)
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn `bash -c <script>` and wire up the dispatcher around it.
#[cfg(unix)]
async fn spawn_script_dispatcher(
    script: &'static str,
) -> (tokio::process::Child, SharedStdin, PendingMap) {
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(script)
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

/// For each request, first emits a server-initiated `roots/list` request that
/// reuses the request's id, then the real response.
#[cfg(unix)]
const COLLIDING_SERVER_REQUEST_SCRIPT: &str = r#"
while IFS= read -r line; do
    id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
    echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"roots/list\",\"params\":{}}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"real\":true}}"
done
"#;

#[cfg(unix)]
#[tokio::test]
async fn server_initiated_request_with_colliding_id_is_not_routed_as_response() {
    use mcp_hub::mcp::protocol::PingRequest;

    let (mut child, stdin_shared, pending) =
        spawn_script_dispatcher(COLLIDING_SERVER_REQUEST_SCRIPT).await;

    let request = PingRequest::new(3);
    let response = send_request(&stdin_shared, &pending, 3, &request, 5).await;

    child.kill().await.ok();

    let response = response.expect("send_request should succeed");
    assert!(response.error.is_none(), "response should not have error");
    assert_eq!(
        response.result,
        Some(serde_json::json!({"real": true})),
        "caller must receive the real result, not the server-initiated request"
    );
}

/// Reads two requests, then answers them in reverse order.
#[cfg(unix)]
const OUT_OF_ORDER_SCRIPT: &str = r#"
IFS= read -r first
IFS= read -r second
id1=$(echo "$first" | sed 's/.*"id":\([0-9]*\).*/\1/')
id2=$(echo "$second" | sed 's/.*"id":\([0-9]*\).*/\1/')
echo "{\"jsonrpc\":\"2.0\",\"id\":$id2,\"result\":{\"for\":$id2}}"
echo "{\"jsonrpc\":\"2.0\",\"id\":$id1,\"result\":{\"for\":$id1}}"
cat > /dev/null
"#;

#[cfg(unix)]
#[tokio::test]
async fn out_of_order_responses_are_routed_by_id() {
    use mcp_hub::mcp::protocol::PingRequest;

    let (mut child, stdin_shared, pending) = spawn_script_dispatcher(OUT_OF_ORDER_SCRIPT).await;

    let mut handles = Vec::new();
    for id in 1u64..=2 {
        let stdin_clone = Arc::clone(&stdin_shared);
        let pending_clone = Arc::clone(&pending);
        handles.push(tokio::spawn(async move {
            let request = PingRequest::new(id);
            send_request(&stdin_clone, &pending_clone, id, &request, 5).await
        }));
        // Make sure request 1 reaches the server before request 2.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    for (index, handle) in handles.into_iter().enumerate() {
        let expected = index as u64 + 1;
        let response = handle
            .await
            .expect("task should not panic")
            .expect("send_request should succeed");
        assert_eq!(
            response.result,
            Some(serde_json::json!({ "for": expected })),
            "request {expected} must receive its own response"
        );
    }

    child.kill().await.ok();
}

/// For each request emits noise (non-JSON, unmatched string id, null id) before
/// the real response.
#[cfg(unix)]
const NOISY_SERVER_SCRIPT: &str = r#"
while IFS= read -r line; do
    id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
    echo "this is not json at all"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"abc\",\"result\":{\"stray\":true}}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"parse\"}}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"real\":$id}}"
done
"#;

#[cfg(unix)]
#[tokio::test]
async fn non_json_and_unmatched_string_id_lines_do_not_stop_the_reader() {
    use mcp_hub::mcp::protocol::PingRequest;

    let (mut child, stdin_shared, pending) = spawn_script_dispatcher(NOISY_SERVER_SCRIPT).await;

    // Two sequential requests: the second only succeeds if the reader survived
    // the noise emitted while handling the first.
    for id in [1u64, 2] {
        let request = PingRequest::new(id);
        let response = send_request(&stdin_shared, &pending, id, &request, 5)
            .await
            .expect("send_request should succeed despite noisy output");
        assert_eq!(
            response.result,
            Some(serde_json::json!({ "real": id })),
            "request {id} must receive the real response"
        );
    }

    child.kill().await.ok();
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
