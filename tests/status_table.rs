/// Unit tests for the enhanced status table (Plan 02-03, Task 5).
///
/// Tests verify that `format_status_table` and `collect_states_from_handles`
/// behave correctly across all Phase 2 columns: Name, State, Health, PID,
/// Uptime, Restarts, Transport.
use std::time::Instant;

use mcp_hub::output::{collect_states_from_handles, format_status_table};
use mcp_hub::types::{HealthStatus, ProcessState, ServerSnapshot};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn running_healthy_snapshot() -> ServerSnapshot {
    ServerSnapshot {
        process_state: ProcessState::Running,
        health: HealthStatus::Healthy {
            latency_ms: 42,
            last_checked: Instant::now(),
        },
        pid: Some(1234),
        uptime_since: Some(Instant::now() - std::time::Duration::from_secs(3661)),
        restart_count: 0,
        transport: "stdio".to_string(),
        ..ServerSnapshot::default()
    }
}

fn backoff_degraded_snapshot() -> ServerSnapshot {
    ServerSnapshot {
        process_state: ProcessState::Backoff {
            attempt: 3,
            until: Instant::now() + std::time::Duration::from_secs(10),
        },
        health: HealthStatus::Degraded {
            consecutive_misses: 3,
            last_success: None,
        },
        pid: None,
        uptime_since: None,
        restart_count: 2,
        transport: "stdio".to_string(),
        ..ServerSnapshot::default()
    }
}

fn fatal_failed_snapshot() -> ServerSnapshot {
    ServerSnapshot {
        process_state: ProcessState::Fatal,
        health: HealthStatus::Failed {
            consecutive_misses: 7,
        },
        pid: None,
        uptime_since: None,
        restart_count: 5,
        transport: "stdio".to_string(),
        ..ServerSnapshot::default()
    }
}

/// Split a rendered table line into trimmed cell texts (borders dropped).
fn cells_of(line: &str) -> Vec<String> {
    line.split(['│', '|'])
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect()
}

/// Look up the cell under `column` in the row whose first cell is `name`,
/// so assertions cannot be satisfied by table borders or other columns.
fn cell(output: &str, name: &str, column: &str) -> String {
    let header = output
        .lines()
        .map(cells_of)
        .find(|c| c.first().map(String::as_str) == Some("Name"))
        .expect("header row");
    let idx = header
        .iter()
        .position(|h| h == column)
        .unwrap_or_else(|| panic!("no column {column}: {output}"));
    let row = output
        .lines()
        .map(cells_of)
        .find(|c| c.first().map(String::as_str) == Some(name))
        .unwrap_or_else(|| panic!("no row for {name}: {output}"));
    row.get(idx)
        .cloned()
        .unwrap_or_else(|| panic!("row too short: {row:?}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Table column presence tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn status_table_has_all_columns() {
    let servers = vec![
        ("alpha".to_string(), running_healthy_snapshot()),
        ("beta".to_string(), backoff_degraded_snapshot()),
    ];

    let output = format_status_table(&servers, false);

    // All 7 column headers must be present.
    assert!(
        output.contains("Name"),
        "Table must contain 'Name' header: {output}"
    );
    assert!(
        output.contains("State"),
        "Table must contain 'State' header: {output}"
    );
    assert!(
        output.contains("Health"),
        "Table must contain 'Health' header: {output}"
    );
    assert!(
        output.contains("PID"),
        "Table must contain 'PID' header: {output}"
    );
    assert!(
        output.contains("Uptime"),
        "Table must contain 'Uptime' header: {output}"
    );
    assert!(
        output.contains("Restarts"),
        "Table must contain 'Restarts' header: {output}"
    );
    assert!(
        output.contains("Transport"),
        "Table must contain 'Transport' header: {output}"
    );
}

#[test]
fn status_table_running_healthy_server() {
    let servers = vec![("my-server".to_string(), running_healthy_snapshot())];

    let output = format_status_table(&servers, false);

    // Server name
    assert!(
        output.contains("my-server"),
        "Table must contain server name: {output}"
    );
    // Process state
    assert!(
        output.contains("running"),
        "Table must contain 'running' state: {output}"
    );
    // Health status
    assert!(
        output.contains("healthy"),
        "Table must contain 'healthy' health: {output}"
    );
    // PID
    assert!(
        output.contains("1234"),
        "Table must contain PID 1234: {output}"
    );
    assert_eq!(cell(&output, "my-server", "PID"), "1234");
    // Uptime in HH:MM:SS format (server ran for 3661 seconds = ~01:01:01)
    let uptime = cell(&output, "my-server", "Uptime");
    assert!(
        uptime.starts_with("01:01:0") && uptime.len() == 8,
        "Uptime cell must be ~01:01:01, got {uptime:?}"
    );
    assert_eq!(cell(&output, "my-server", "Restarts"), "0");
    // Transport
    assert!(
        output.contains("stdio"),
        "Table must contain transport 'stdio': {output}"
    );
}

#[test]
fn status_table_fatal_failed_server() {
    let servers = vec![("crashed".to_string(), fatal_failed_snapshot())];

    let output = format_status_table(&servers, false);

    assert!(
        output.contains("crashed"),
        "Table must contain server name: {output}"
    );
    assert!(
        output.contains("fatal"),
        "Table must contain 'fatal' state: {output}"
    );
    assert!(
        output.contains("failed"),
        "Table must contain 'failed' health: {output}"
    );
    // No PID and no uptime — each shows the dash placeholder in its own cell.
    assert_eq!(cell(&output, "crashed", "PID"), "-");
    assert_eq!(cell(&output, "crashed", "Uptime"), "-");
    assert_eq!(cell(&output, "crashed", "Restarts"), "5");
}

#[test]
fn status_table_empty_servers() {
    // Empty slice should print header only without panicking.
    let servers: Vec<(String, ServerSnapshot)> = vec![];
    let output = format_status_table(&servers, false);

    // Header must still be present.
    assert!(
        output.contains("Name"),
        "Empty table must still have header: {output}"
    );
    assert!(
        output.contains("Health"),
        "Empty table must still have Health header: {output}"
    );
}

#[test]
fn status_table_color_disabled() {
    let servers = vec![("srv".to_string(), running_healthy_snapshot())];

    // Must not panic with color=false.
    let output = format_status_table(&servers, false);
    assert!(!output.is_empty(), "Table output must not be empty");
}

#[test]
fn status_table_color_enabled() {
    let servers = vec![("srv".to_string(), running_healthy_snapshot())];

    // Must not panic with color=true (even if terminal is not a TTY in CI).
    let output = format_status_table(&servers, true);
    assert!(!output.is_empty(), "Colored table output must not be empty");
}

// ─────────────────────────────────────────────────────────────────────────────
// collect_states_from_handles
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn collect_states_snapshot_format() {
    // Build mock ServerHandles with seeded watch channels.
    let (tx1, rx1) = tokio::sync::watch::channel(running_healthy_snapshot());
    let (tx2, rx2) = tokio::sync::watch::channel(fatal_failed_snapshot());

    // Avoid "unused" warnings for the senders — they must stay alive.
    let _tx1 = tx1;
    let _tx2 = tx2;

    let (cmd_tx1, _cmd_rx1) = tokio::sync::mpsc::channel(1);
    let (cmd_tx2, _cmd_rx2) = tokio::sync::mpsc::channel(1);

    let handles = vec![
        mcp_hub::supervisor::ServerHandle {
            name: "alpha".to_string(),
            state_rx: rx1,
            cmd_tx: cmd_tx1,
            task: tokio::task::spawn(async {}),
        },
        mcp_hub::supervisor::ServerHandle {
            name: "beta".to_string(),
            state_rx: rx2,
            cmd_tx: cmd_tx2,
            task: tokio::task::spawn(async {}),
        },
    ];

    let states = collect_states_from_handles(&handles);

    assert_eq!(states.len(), 2, "Must return one entry per handle");

    let (name_a, snap_a) = &states[0];
    assert_eq!(name_a, "alpha");
    assert!(
        matches!(snap_a.process_state, ProcessState::Running),
        "alpha must be Running"
    );

    let (name_b, snap_b) = &states[1];
    assert_eq!(name_b, "beta");
    assert!(
        matches!(snap_b.process_state, ProcessState::Fatal),
        "beta must be Fatal"
    );
}
