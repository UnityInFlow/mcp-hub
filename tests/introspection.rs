//! Introspection integration tests — covers the full MCP capability discovery
//! sequence: initialize handshake, notifications/initialized, and concurrent
//! tools/list + resources/list + prompts/list.
//!
//! Tests use a bash-based mock MCP server (tests/fixtures/mock-mcp-server.sh)
//! that handles all relevant methods with configurable behaviour via env vars.

#[cfg(unix)]
mod unix_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use mcp_hub::mcp::dispatcher::{reader_task, IdAllocator, PendingMap, SharedStdin};
    use mcp_hub::mcp::introspect::run_introspection;
    use mcp_hub::types::ServerSnapshot;
    use tokio::sync::Mutex;

    // ─────────────────────────────────────────────────────────────────────────────
    // Test fixture helper
    // ─────────────────────────────────────────────────────────────────────────────

    /// Path to the mock MCP server script relative to the workspace root.
    fn mock_server_script() -> &'static str {
        "tests/fixtures/mock-mcp-server.sh"
    }

    /// Spawn the mock MCP server and wire up the dispatcher infrastructure.
    ///
    /// Returns (child, SharedStdin, PendingMap, IdAllocator).
    async fn spawn_mock_server(
        env: &[(&str, &str)],
    ) -> (
        tokio::process::Child,
        SharedStdin,
        PendingMap,
        Arc<IdAllocator>,
    ) {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg(mock_server_script())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        for (key, val) in env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .expect("bash and mock-mcp-server.sh should be available");

        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");

        let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let stdin_for_reader = Arc::clone(&stdin_shared);
        let pending_clone = Arc::clone(&pending);
        tokio::spawn(async move {
            reader_task(stdout, pending_clone, stdin_for_reader).await;
        });

        let id_alloc = Arc::new(IdAllocator::new());

        (child, stdin_shared, pending, id_alloc)
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Cancellation drops an in-flight introspection (issue #28)
    // ─────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn introspection_is_dropped_promptly_on_cancel() {
        use mcp_hub::mcp::introspect::run_introspection_until_cancelled;
        use tokio_util::sync::CancellationToken;

        // Reads stdin but never answers, so `initialize` waits for its 10 s timeout.
        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg("cat > /dev/null")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("bash should be available");
        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (p, s) = (Arc::clone(&pending), Arc::clone(&stdin_shared));
        tokio::spawn(async move { reader_task(stdout, p, s).await });

        let id_alloc = Arc::new(IdAllocator::new());
        let (tx, rx) = tokio::sync::watch::channel(ServerSnapshot::default());
        let cancel = CancellationToken::new();

        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            canceller.cancel();
        });

        let start = std::time::Instant::now();
        let result = run_introspection_until_cancelled(
            "slow",
            &stdin_shared,
            &pending,
            &id_alloc,
            &tx,
            &cancel,
        )
        .await;
        let elapsed = start.elapsed();

        child.kill().await.ok();

        assert!(result.is_none(), "cancelled introspection must yield None");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "cancel must not wait for the request timeout, took {elapsed:?}"
        );
        assert!(
            rx.borrow().capabilities.introspected_at.is_none(),
            "no capabilities may be written after cancel"
        );
        // The dropped request must not leave its pending entry behind.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(pending.lock().await.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Test 1: successful introspection captures correct counts
    // ─────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn introspection_captures_correct_counts() {
        let (mut child, stdin_shared, pending, id_alloc) = spawn_mock_server(&[]).await;

        let initial_snapshot = ServerSnapshot::default();
        let (tx, rx) = tokio::sync::watch::channel(initial_snapshot);

        let result =
            run_introspection("test-server", &stdin_shared, &pending, &id_alloc, &tx).await;

        child.kill().await.ok();

        assert!(
            result.is_ok(),
            "run_introspection should succeed with a full-featured mock server, got: {:?}",
            result
        );

        let caps = result.unwrap();

        // Mock server returns 2 tools, 1 resource, 1 prompt.
        assert_eq!(
            caps.tools.len(),
            2,
            "expected 2 tools, got {}",
            caps.tools.len()
        );
        assert_eq!(
            caps.resources.len(),
            1,
            "expected 1 resource, got {}",
            caps.resources.len()
        );
        assert_eq!(
            caps.prompts.len(),
            1,
            "expected 1 prompt, got {}",
            caps.prompts.len()
        );
        assert!(
            caps.introspected_at.is_some(),
            "introspected_at should be set after successful introspection"
        );

        // Verify the snapshot watch channel reflects the updated capabilities.
        let snapshot = rx.borrow().clone();
        assert_eq!(
            snapshot.capabilities.tools.len(),
            2,
            "snapshot should reflect 2 tools"
        );
        assert_eq!(
            snapshot.capabilities.resources.len(),
            1,
            "snapshot should reflect 1 resource"
        );
        assert_eq!(
            snapshot.capabilities.prompts.len(),
            1,
            "snapshot should reflect 1 prompt"
        );
        assert!(
            snapshot.capabilities.introspected_at.is_some(),
            "snapshot introspected_at should be set"
        );

        // Verify tool names.
        let tool_names: Vec<&str> = caps.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            tool_names.contains(&"search"),
            "expected 'search' tool, got: {:?}",
            tool_names
        );
        assert!(
            tool_names.contains(&"fetch"),
            "expected 'fetch' tool, got: {:?}",
            tool_names
        );
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Test 2: server missing resources capability
    // ─────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn introspection_skips_unsupported_capability() {
        // MOCK_SKIP_RESOURCES=1 causes the mock to omit "resources" from initialize.
        let (mut child, stdin_shared, pending, id_alloc) =
            spawn_mock_server(&[("MOCK_SKIP_RESOURCES", "1")]).await;

        let initial_snapshot = ServerSnapshot::default();
        let (tx, _rx) = tokio::sync::watch::channel(initial_snapshot);

        let result = run_introspection(
            "test-server-no-resources",
            &stdin_shared,
            &pending,
            &id_alloc,
            &tx,
        )
        .await;

        child.kill().await.ok();

        assert!(
            result.is_ok(),
            "run_introspection should succeed even when resources are unsupported, got: {:?}",
            result
        );

        let caps = result.unwrap();

        // Tools and prompts should be populated; resources skipped (not declared).
        assert_eq!(
            caps.tools.len(),
            2,
            "tools should still be populated: {:?}",
            caps.tools
        );
        assert_eq!(
            caps.resources.len(),
            0,
            "resources should be empty when capability not declared"
        );
        assert_eq!(
            caps.prompts.len(),
            1,
            "prompts should still be populated: {:?}",
            caps.prompts
        );
        assert!(
            caps.introspected_at.is_some(),
            "introspected_at should be set"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Test 3: server returns error on tools/list
    // ─────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn introspection_handles_error_response() {
        // MOCK_TOOLS_ERROR=1 causes the mock to return an error on tools/list.
        let (mut child, stdin_shared, pending, id_alloc) =
            spawn_mock_server(&[("MOCK_TOOLS_ERROR", "1")]).await;

        let initial_snapshot = ServerSnapshot::default();
        let (tx, _rx) = tokio::sync::watch::channel(initial_snapshot);

        let result = run_introspection(
            "test-server-tools-error",
            &stdin_shared,
            &pending,
            &id_alloc,
            &tx,
        )
        .await;

        child.kill().await.ok();

        // run_introspection should succeed overall (errors on individual lists are non-fatal).
        assert!(
            result.is_ok(),
            "run_introspection should not propagate tools/list error, got: {:?}",
            result
        );

        let caps = result.unwrap();

        // tools should be empty (error response), resources and prompts still populated.
        assert_eq!(
            caps.tools.len(),
            0,
            "tools should be empty after error response"
        );
        assert_eq!(
            caps.resources.len(),
            1,
            "resources should be populated despite tools error"
        );
        assert_eq!(
            caps.prompts.len(),
            1,
            "prompts should be populated despite tools error"
        );
        // introspected_at is set even if some lists failed.
        assert!(
            caps.introspected_at.is_some(),
            "introspected_at should be set"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Test 4: introspection timeouts
    // ─────────────────────────────────────────────────────────────────────────────

    /// A server that reads stdin but never answers: `initialize` must fail with
    /// a timeout error after the 10 s request timeout (virtual time, so the
    /// test is instant and the silent child cannot race the clock).
    #[tokio::test(start_paused = true)]
    async fn introspection_times_out_when_initialize_is_never_answered() {
        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg("cat > /dev/null")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("bash should be available");
        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (p, s) = (Arc::clone(&pending), Arc::clone(&stdin_shared));
        tokio::spawn(async move { reader_task(stdout, p, s).await });

        let id_alloc = Arc::new(IdAllocator::new());
        let (tx, rx) = tokio::sync::watch::channel(ServerSnapshot::default());

        let start = tokio::time::Instant::now();
        let result = run_introspection("silent", &stdin_shared, &pending, &id_alloc, &tx).await;
        let elapsed = start.elapsed();

        child.kill().await.ok();

        let err = result.expect_err("an unanswered initialize must be an error");
        assert!(
            err.to_string().to_lowercase().contains("time"),
            "error should mention the timeout, got: {err}"
        );
        assert!(
            elapsed >= std::time::Duration::from_secs(10)
                && elapsed <= std::time::Duration::from_secs(12),
            "should fail at the 10 s request timeout, took {elapsed:?}"
        );
        assert!(
            rx.borrow().capabilities.introspected_at.is_none(),
            "no capabilities may be recorded after a failed handshake"
        );
    }

    /// `initialize` succeeds but the list requests are never answered
    /// (MOCK_SILENT_LISTS): each times out after 10 s, and introspection
    /// degrades to empty capabilities instead of failing or hanging.
    #[tokio::test]
    async fn introspection_list_timeouts_yield_empty_capabilities() {
        let (mut child, stdin_shared, pending, id_alloc) =
            spawn_mock_server(&[("MOCK_SILENT_LISTS", "1")]).await;
        let (tx, _rx) = tokio::sync::watch::channel(ServerSnapshot::default());

        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            run_introspection("silent-lists", &stdin_shared, &pending, &id_alloc, &tx),
        )
        .await;
        let elapsed = start.elapsed();

        child.kill().await.ok();
        child.wait().await.ok();

        let caps = result
            .expect("introspection must not hang past the request timeout")
            .expect("list timeouts are non-fatal");
        assert!(caps.tools.is_empty() && caps.resources.is_empty() && caps.prompts.is_empty());
        assert!(
            elapsed >= std::time::Duration::from_secs(9),
            "lists should have waited for their 10 s timeout, took {elapsed:?}"
        );
        assert!(caps.introspected_at.is_some());
    }

    /// The server answers `initialize` and then exits. Depending on whether the
    /// pipe is already closed when `notifications/initialized` is written this
    /// is a prompt Err (broken pipe) or an Ok with empty capabilities (the list
    /// requests fail fast); either way it must neither hang nor report any
    /// capability.
    #[tokio::test]
    async fn introspection_fails_fast_when_server_exits_after_initialize() {
        let script = r#"
line=$(head -1)
method=$(echo "$line" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('method',''))" 2>/dev/null)
id=$(echo "$line" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('id',''))" 2>/dev/null)
if [ "$method" = "initialize" ]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{},\"resources\":{},\"prompts\":{}},\"serverInfo\":{\"name\":\"crash-mock\",\"version\":\"1.0\"}}}"
fi
# Exit immediately after initialize — list requests will get RecvError (stream closed).
exit 0
"#;

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("bash should be available");

        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");

        let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let stdin_for_reader = Arc::clone(&stdin_shared);
        let pending_clone = Arc::clone(&pending);
        tokio::spawn(async move {
            reader_task(stdout, pending_clone, stdin_for_reader).await;
        });

        let id_alloc = Arc::new(IdAllocator::new());
        let initial_snapshot = ServerSnapshot::default();
        let (tx, rx) = tokio::sync::watch::channel(initial_snapshot);

        let start = std::time::Instant::now();

        // run_introspection should complete (not hang) when the server crashes
        // mid-introspection. List requests get RecvError and return empty vecs.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_introspection("crash-mock", &stdin_shared, &pending, &id_alloc, &tx),
        )
        .await;

        let elapsed = start.elapsed();

        child.kill().await.ok();
        child.wait().await.ok();

        // Should complete without hanging (within the 15s outer timeout).
        assert!(
            result.is_ok(),
            "introspection should complete within 15 seconds, but timed out"
        );

        assert!(
            elapsed.as_secs() < 14,
            "introspection should complete well within 14 seconds, took {}s",
            elapsed.as_secs()
        );

        let outcome = result.expect("introspection must not hang");
        if let Ok(caps) = outcome {
            assert!(caps.tools.is_empty(), "tools: {:?}", caps.tools);
            assert!(caps.resources.is_empty(), "resources: {:?}", caps.resources);
            assert!(caps.prompts.is_empty(), "prompts: {:?}", caps.prompts);
        }
        assert!(
            rx.borrow().capabilities.tools.is_empty(),
            "no tools may be recorded for a server that exited"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Non-unix stubs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(unix))]
#[test]
fn introspection_tests_require_unix() {
    // bash-based integration tests are Unix-only.
    // On Windows, these would need a PowerShell or cmd equivalent.
}
