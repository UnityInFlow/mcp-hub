---
plan_id: "02-02"
title: "MCP Client + Health Monitor"
status: complete
date_completed: 2026-04-02
---

# Plan 02-02: MCP Client + Health Monitor — Execution Summary

## What Was Built

7 tasks completed and committed. The MCP health monitoring layer is now fully integrated into the supervisor.

---

## Tasks Completed

### Task 1 — SpawnedProcess stdin field
- Added `pub stdin: Option<ChildStdin>` to `SpawnedProcess`
- Added `use tokio::process::ChildStdin` import
- `spawn_server` now captures `child.stdin.take()` and stores it alongside stdout
- **Commit:** `feat(02-02-01): add stdin field to SpawnedProcess for MCP ping writes`

### Task 2 — Stderr wired to LogAggregator
- `spawn_server` now accepts `log_agg: Option<Arc<LogAggregator>>`
- When `Some(agg)` is provided: stderr lines pushed to ring buffer, formatted, and printed to terminal
- When `None`: falls back to `tracing::debug!` drain (backward compat for tests)
- `run_server_supervisor` and `start_all_servers` updated to accept and thread `Arc<LogAggregator>`
- Existing supervisor tests updated to pass `None` for `log_agg`
- **Commit:** `feat(02-02-02): wire stderr to LogAggregator; update spawn_server, run_server_supervisor, start_all_servers signatures`

### Task 3 — Watch channel upgraded to ServerSnapshot
- `state_tx` and `state_rx` now use `watch::Sender/Receiver<ServerSnapshot>` instead of `(ProcessState, Option<u32>)`
- All `state_tx.send(...)` replaced with `state_tx.send_modify(|s| { ... })` for in-place mutation
- `total_restarts: u32` local counter added; increments on crash and on explicit Restart; stored in snapshot
- `health` field set to `HealthStatus::Unknown` on Starting and Running transitions (D-04)
- `uptime_since` set to `Some(Instant::now())` on Running
- `ServerHandle.state_rx` type updated
- `start_all_servers` initializes watch channel with `ServerSnapshot { transport, ..Default::default() }`
- `wait_for_initial_states` updated to use `.process_state.clone()`
- `output.rs`: `collect_states_from_handles` returns `Vec<(String, ServerSnapshot)>`, `print_status_table` updated to use snapshot (now shows Health column)
- **Commit:** `feat(02-02-03): upgrade watch channel from (ProcessState, Option<u32>) to ServerSnapshot; add total_restarts tracking`

### Task 4 — Health check loop (MCP ping task) — `src/mcp/health.rs`
- `pub async fn ping_server(stdin, stdout_reader, id) -> anyhow::Result<u64>`: serializes PingRequest, writes to stdin, reads stdout with 5-second timeout, drains non-matching lines
- `pub async fn run_health_check_loop(...)`: uses `tokio::time::interval` with `MissedTickBehavior::Skip`, tracks `consecutive_misses`, calls `compute_health_status` on failure, logs health state transitions
- `pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS: u64 = 30`
- `src/mcp/mod.rs` updated with `pub mod health`
- No `unwrap()` in production code
- **Committed as part of Task 3's commit** (created to satisfy dependency)

### Task 5 — Health task integration in supervisor lifecycle
- Supervisor now extracts `spawned_stdin` and `spawned_stdout` before the select loop
- Spawns `run_health_check_loop` task if both stdin and stdout are available
- `health_cancel = shutdown.child_token()` created per-spawn; cancelled in:
  - crash branch (before backoff wait)
  - shutdown branch (before `shutdown_process`)
  - restart branch (before `shutdown_process`)
  - shutdown command branch
- `HealthStatus::Unknown` set on Starting and Running state transitions
- Configurable interval from `config.health_check_interval.unwrap_or(DEFAULT_HEALTH_CHECK_INTERVAL_SECS)`
- Fallback warning logged if stdin or stdout is missing (health checks disabled)
- **Committed as part of Task 3's commit**

### Task 6 — main.rs wires LogAggregator
- `Arc<LogAggregator>` created before `start_all_servers` with `capacity=10_000` per server
- `Arc::clone(&log_agg)` passed to `start_all_servers` and `run_foreground_loop`
- `run_foreground_loop` signature updated with `_log_agg: Arc<logs::LogAggregator>` (stored for Plan 02-03 `logs` command)
- `use std::sync::Arc` import added
- **Commit:** `feat(02-02-06): create LogAggregator in main.rs, wire through start_all_servers and run_foreground_loop`

### Task 7 — Health monitor tests (`tests/health_monitor.rs`)
- **Serialization tests**: `ping_request_serialization`, `ping_request_id_zero`, `ping_request_large_id`
- **Deserialization tests**: `successful_ping_response`, `error_ping_response`, `ping_response_missing_result_field`
- **Constant test**: `default_health_check_interval_is_reasonable`
- **Integration tests** (`#[cfg(unix)]`):
  - `ping_server_returns_latency_for_valid_responder`: spawns bash ping-responder script, verifies Ok(latency < 5000ms)
  - `ping_timeout`: spawns `cat > /dev/null`, verifies error within 8 seconds
  - `run_health_check_loop_sets_healthy_on_first_ping`: verifies watch channel receives `HealthStatus::Healthy` after first ping
  - `run_health_check_loop_exits_on_cancel`: verifies loop exits cleanly within 1 second on cancellation
- Non-unix stub provided
- **Commit:** `feat(02-02-07): add health monitor tests; ping serialization, response deserialization, ping_server integration, timeout, loop cancel`

---

## Verification Results

```
cargo build      — 0 errors, 0 warnings
cargo test       — 66 passed (9 suites)
cargo clippy     — No issues found (-D warnings)
cargo fmt --check — Clean
```

---

## Key Design Decisions Made During Execution

1. **health.rs created early** (Task 3 commit) to resolve the supervisor dependency. The plan's task ordering was Task 4 → Task 5, but since Task 3 integrated health check spawning into the supervisor rewrite, health.rs was created concurrently.

2. **`total_restarts` never resets** — counts both crash-based restarts and explicit Restart commands, stored in every snapshot send.

3. **`uptime_since` uses `Instant::now()` in the `Running` state transition** via `send_modify` closure, ensuring accurate uptime from the moment the state is set.

4. **`print_status_table` signature change** — now accepts `&[(String, ServerSnapshot)]` and renders a 4-column table (Name, State, PID, Health). This is a non-breaking change since `collect_states_from_handles` is the only caller.

5. **Fallback when stdin/stdout unavailable** — logs a warning and skips health checks (does not crash). This handles edge cases like HTTP transport servers.

---

## Files Modified

| File | Change |
|---|---|
| `src/supervisor.rs` | SpawnedProcess stdin, spawn_server signature, stderr→LogAgg, ServerSnapshot watch channel, health task spawn, health_cancel pattern, total_restarts |
| `src/mcp/health.rs` | New file — ping_server, run_health_check_loop, DEFAULT_HEALTH_CHECK_INTERVAL_SECS |
| `src/mcp/mod.rs` | Added `pub mod health` |
| `src/output.rs` | collect_states_from_handles returns Vec<(String, ServerSnapshot)>, print_status_table updated |
| `src/main.rs` | LogAggregator creation, Arc import, run_foreground_loop signature |
| `tests/health_monitor.rs` | New file — 11 tests |
| `tests/supervisor_test.rs` | Updated 3 spawn_server calls to pass None for log_agg |

---

## Requirements Addressed

- **PROC-04**: stderr piped and drained (now routed to LogAggregator)
- **HLTH-01**: Health check loop with JSON-RPC ping
- **HLTH-02**: HealthStatus transitions (Unknown → Healthy → Degraded → Failed)
- **HLTH-04**: 5-second timeout per ping
- **HLTH-05**: Configurable interval (default 30s, override via `health_check_interval` in config)
