/// Full Phase 2 integration tests (Plan 02-03, Task 7).
///
/// These tests exercise the complete Phase 2 feature set end-to-end:
/// - Log aggregation from server stderr
/// - Health status transitions (Healthy, Degraded)
/// - Status table output with all 7 columns
///
/// All tests on Unix only. All tests clean up child processes via
/// CancellationToken + stop_all_servers with a tokio::time::timeout guard.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mcp_hub::config::{HubConfig, ServerConfig};
use mcp_hub::logs::LogAggregator;
use mcp_hub::supervisor::{start_all_servers, stop_all_servers};
use mcp_hub::types::HealthStatus;
use tokio_util::sync::CancellationToken;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a minimal HubConfig with a single server entry.
fn single_server_config(name: &str, command: &str, args: Vec<&str>) -> HubConfig {
    let server = ServerConfig {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
        env_file: None,
        transport: "stdio".to_string(),
        cwd: None,
        health_check_interval: None,
        max_retries: None,
        restart_delay: None,
    };

    let mut servers = HashMap::new();
    servers.insert(name.to_string(), server);
    HubConfig {
        hub: Default::default(),
        servers,
    }
}

/// Build a config with a custom health_check_interval (in seconds).
fn single_server_config_with_health(
    name: &str,
    command: &str,
    args: Vec<&str>,
    health_interval: u64,
) -> HubConfig {
    let server = ServerConfig {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
        env_file: None,
        transport: "stdio".to_string(),
        cwd: None,
        health_check_interval: Some(health_interval),
        max_retries: None,
        restart_delay: None,
    };

    let mut servers = HashMap::new();
    servers.insert(name.to_string(), server);
    HubConfig {
        hub: Default::default(),
        servers,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: log lines from stderr are captured in the LogAggregator
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn test_log_aggregation_from_stderr() {
    let server_name = "log-test-server";
    let config = single_server_config(
        server_name,
        "bash",
        vec![
            "-c",
            "echo 'log line 1' >&2; echo 'log line 2' >&2; sleep 60",
        ],
    );

    let shutdown = CancellationToken::new();
    let server_names: Vec<String> = config.servers.keys().cloned().collect();
    let log_agg = Arc::new(LogAggregator::new(&server_names, 10_000));

    let handles = start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

    // Wait for stderr lines to be captured (bash writes them immediately on start).
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Retrieve lines from the buffer.
    let buf = log_agg
        .get_buffer(server_name)
        .expect("LogAggregator should have a buffer for the server");
    let lines = buf.snapshot().await;

    // Clean up before asserting so the process always gets stopped.
    shutdown.cancel();
    let timeout_result =
        tokio::time::timeout(Duration::from_secs(10), stop_all_servers(handles)).await;
    assert!(
        timeout_result.is_ok(),
        "stop_all_servers should complete within 10s"
    );

    assert!(
        lines.len() >= 2,
        "LogAggregator should have captured at least 2 lines from stderr, got: {}",
        lines.len()
    );

    // Each log line must reference the correct server name.
    for line in &lines {
        assert_eq!(
            line.server, server_name,
            "LogLine.server must match the server name"
        );
    }

    // The captured messages should include our test strings.
    let messages: Vec<&str> = lines.iter().map(|l| l.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("log line 1")),
        "Must capture 'log line 1', got: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("log line 2")),
        "Must capture 'log line 2', got: {messages:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: health check transitions server to Healthy when it responds to pings
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn test_health_transitions_to_healthy() {
    // Use the ping-responder.sh fixture — it responds to JSON-RPC pings on stdin.
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ping-responder.sh"
    );

    let config = single_server_config_with_health("ping-server", fixture, vec![], 1);

    let shutdown = CancellationToken::new();
    let server_names: Vec<String> = config.servers.keys().cloned().collect();
    let log_agg = Arc::new(LogAggregator::new(&server_names, 10_000));

    let handles = start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

    // Wait long enough for at least one health check to complete (1s interval + margin).
    tokio::time::sleep(Duration::from_secs(3)).await;

    let snapshot = handles[0].state_rx.borrow().clone();

    // Clean up before asserting.
    shutdown.cancel();
    let timeout_result =
        tokio::time::timeout(Duration::from_secs(10), stop_all_servers(handles)).await;
    assert!(
        timeout_result.is_ok(),
        "stop_all_servers should complete within 10s"
    );

    assert!(
        matches!(snapshot.health, HealthStatus::Healthy { .. }),
        "Health must be Healthy after responding to pings, got: {:?}",
        snapshot.health
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: health degrades when server is unresponsive to pings
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn test_health_degrades_on_unresponsive_server() {
    // A server that reads stdin but never writes to stdout — pings time out.
    let config =
        single_server_config_with_health("silent-server", "bash", vec!["-c", "cat > /dev/null"], 1);

    let shutdown = CancellationToken::new();
    let server_names: Vec<String> = config.servers.keys().cloned().collect();
    let log_agg = Arc::new(LogAggregator::new(&server_names, 10_000));

    let handles = start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

    // Wait long enough for 2+ missed pings (1s interval, 5s ping timeout each).
    // We need at least 2 consecutive misses to transition away from Unknown.
    // Each ping attempt takes up to 5s to timeout, so wait ~12s to be safe.
    tokio::time::sleep(Duration::from_secs(12)).await;

    let snapshot = handles[0].state_rx.borrow().clone();

    // Clean up before asserting.
    shutdown.cancel();
    let timeout_result =
        tokio::time::timeout(Duration::from_secs(10), stop_all_servers(handles)).await;
    assert!(
        timeout_result.is_ok(),
        "stop_all_servers should complete within 10s"
    );

    // The server should be Degraded (2–6 misses) or Failed (7+ misses) — not Healthy or Unknown.
    let is_degraded_or_worse = matches!(
        snapshot.health,
        HealthStatus::Degraded { .. } | HealthStatus::Failed { .. }
    );
    assert!(
        is_degraded_or_worse,
        "Health must be Degraded or Failed for unresponsive server, got: {:?}",
        snapshot.health
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: status table output has 7 columns
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_status_table_output_has_seven_columns() {
    use mcp_hub::output::format_status_table;
    use mcp_hub::types::{ProcessState, ServerSnapshot};

    let snapshot = ServerSnapshot {
        process_state: ProcessState::Running,
        health: HealthStatus::Healthy {
            latency_ms: 10,
            last_checked: std::time::Instant::now(),
        },
        pid: Some(999),
        uptime_since: Some(std::time::Instant::now()),
        restart_count: 0,
        transport: "stdio".to_string(),
        ..ServerSnapshot::default()
    };

    let servers = vec![("test-server".to_string(), snapshot)];
    let output = format_status_table(&servers, false);

    // All 7 column headers must appear in the rendered table.
    let required_headers = [
        "Name",
        "State",
        "Health",
        "PID",
        "Uptime",
        "Restarts",
        "Transport",
    ];
    for header in required_headers {
        assert!(
            output.contains(header),
            "Status table must contain column '{header}': {output}"
        );
    }
}
