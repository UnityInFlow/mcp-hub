---
plan_id: "02-03"
title: "Status Command + Logs Command + Integration"
status: completed
date: 2026-04-02
commits:
  - ee60d86
  - 649ee1a
  - 4025a8d
  - 6e1c85a
  - 2474f03
---

# Summary: Plan 02-03 — Status Command + Logs Command + Integration

## What Was Done

### Task 1: Enhanced status table (src/output.rs)

- Replaced 4-column table with 7-column table: **Name, State, Health, PID, Uptime, Restarts, Transport** (D-09).
- Added `format_status_table` function that returns a `String` (testable without capturing stdout).
- `print_status_table` is now a thin wrapper around `format_status_table`.
- Health column is colored: Healthy=Green, Degraded=Yellow, Failed=Red, Unknown=DarkGrey.
- Uptime column uses `format_uptime(since.elapsed())` → `HH:MM:SS` format (D-10).
- `collect_states_from_handles` was already correct — returns `Vec<(String, ServerSnapshot)>`.

### Task 2: Logs and Status CLI subcommands (src/cli.rs)

- Added `LogsArgs` struct with `--follow` (`-f`), `--server` (`-s`), and `--lines` (`-n`, default 100).
- Added `Commands::Status` and `Commands::Logs(LogsArgs)` variants to the `Commands` enum.

### Task 3: CLI handlers in main.rs

- `Commands::Status` prints "no daemon running" message and exits with code 1.
- `Commands::Logs` prints appropriate "no daemon running" or "requires daemon mode" message and exits with code 1.
- Both handlers are Phase 2 stubs; full IPC will be wired in Phase 3.

### Task 4: Foreground loop logs/status stdin commands (src/main.rs)

- Updated `run_foreground_loop` signature to accept `Arc<LogAggregator>` (no longer `_log_agg`).
- Updated `handle_stdin_command` signature to accept `&logs::LogAggregator`.
- Added `logs` stdin command: dumps last 100 lines from all servers merged and sorted by timestamp.
- Added `logs <name>` stdin command: dumps last 100 lines for a specific server.
- Updated `help` command to list both `logs` and `logs <name>` entries.
- Fixed clippy `deref-addrof` warning: `&*log_agg` → `&log_agg`.

### Task 5: Status table unit tests (tests/status_table.rs)

7 tests — all passing:
- `status_table_has_all_columns` — verifies all 7 headers present
- `status_table_running_healthy_server` — verifies name, state, health, PID, uptime, restarts, transport
- `status_table_fatal_failed_server` — verifies fatal/failed server rendering with dash placeholders
- `status_table_empty_servers` — verifies header-only table for empty input
- `status_table_color_disabled` — no-panic with color=false
- `status_table_color_enabled` — no-panic with color=true
- `collect_states_snapshot_format` — verifies collect_states_from_handles returns correct snapshots

### Task 6: Logs command tests (tests/logs_command.rs)

7 tests — all passing:
- `logs_subcommand_prints_daemon_required` — exit 1, stderr "no daemon running"
- `logs_follow_subcommand_prints_daemon_required` — exit 1, stderr "requires daemon mode"
- `logs_subcommand_with_server_filter` — exit 1 with --server flag
- `status_subcommand_prints_daemon_required` — exit 1, stderr "no daemon running"
- `logs_args_parsing` — short flags (-f, -s, -n) parsed correctly
- `logs_args_defaults` — default values (follow=false, server=None, lines=100)
- `logs_args_long_flags` — long flags (--follow, --server, --lines) parsed correctly

### Task 7: Phase 2 integration tests (tests/integration_phase2.rs)

4 tests — all passing:
- `test_log_aggregation_from_stderr` — bash server emits 2 stderr lines; LogAggregator captures them with correct server name
- `test_health_transitions_to_healthy` — ping-responder.sh responds to pings; health transitions to Healthy within 3s
- `test_health_degrades_on_unresponsive_server` — `cat > /dev/null` server never responds; health degrades to Degraded or Failed within 12s
- `test_status_table_output_has_seven_columns` — format_status_table output contains all 7 column headers

New fixture: `tests/fixtures/ping-responder.sh` — minimal MCP server that responds to JSON-RPC pings, using python3 to parse the id field.

## Files Modified

| File | Change |
|---|---|
| `src/output.rs` | Added `format_status_table` with 7 columns and health colors; `print_status_table` is thin wrapper |
| `src/cli.rs` | Added `LogsArgs` struct; added `Commands::Status` and `Commands::Logs` variants |
| `src/main.rs` | Added Status/Logs CLI handlers; updated foreground loop with logs stdin command |
| `tests/status_table.rs` | New — 7 unit tests for status table rendering and collect_states_from_handles |
| `tests/logs_command.rs` | New — 7 tests for logs/status CLI subcommands and LogsArgs parsing |
| `tests/integration_phase2.rs` | New — 4 integration tests covering full Phase 2 feature set |
| `tests/fixtures/ping-responder.sh` | New — minimal MCP ping-responder fixture script |

## Verification Results

```
cargo build       — clean (0 errors)
cargo clippy      — clean (0 warnings, -D warnings)
cargo fmt --check — clean
cargo test        — 84 passed (12 suites, 32s)
```

## Requirements Addressed

- **PROC-04**: Status table displays 7 columns with correct data per column.
- **LOG-01**: `logs` stdin command dumps ring buffer with timestamp+server prefix.
- **LOG-02**: `logs <name>` stdin command filters to specific server's buffer.
- All Phase 1 tests continue to pass — no regressions.
