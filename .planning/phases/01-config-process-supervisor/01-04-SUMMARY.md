---
plan_id: "01-04"
title: "GAP-01 closure: stdin restart command for foreground mode"
phase: 1
wave: 4
status: complete
date: 2026-04-02
---

# Plan 04 Execution Summary — Stdin Restart Command (GAP-01 Closure)

## Objective

Close GAP-01 (PROC-03: "User can restart a specific server by name") by adding an
interactive stdin command reader to the foreground hub loop. This allows users to type
`restart <name>`, `status`, or `help` while the hub is running in a terminal.

---

## Tasks Executed

### Task 1 — Add stdin command reader to the foreground hub loop

**Files modified:** `src/main.rs`

**What was done:**
- Replaced the bare `wait_for_shutdown_signal().await?` call in `Commands::Start` with a
  call to the new `run_foreground_loop` async function.
- Added `run_foreground_loop`: uses `tokio::io::BufReader` on stdin and `tokio::select!`
  to concurrently read stdin lines and wait for Ctrl+C / SIGTERM. Platform-conditional
  compilation (`#[cfg(unix)]` / `#[cfg(not(unix))]`) is applied at the block level
  (not inside `select!` arms) to avoid macro limitations.
- Added `handle_stdin_command`: dispatches stdin input to `restart <name>`, `status`,
  `help`, and unknown-command handlers.
- When stdin is closed (piped input exhausted), the hub falls back to
  `wait_for_shutdown_signal()` rather than exiting.

**Key design decisions:**
- SIGTERM signal handler is installed once before the loop (not re-registered on each
  iteration), avoiding races.
- `handle_stdin_command` calls `supervisor::restart_server`, sleeps 2 seconds, then
  reprints the status table via `output::collect_states_from_handles` and
  `output::print_status_table`.

**Commit:** `feat(01-04): add run_foreground_loop and handle_stdin_command to main.rs`

---

### Task 2 — Remove dead_code allows from restart_server and SupervisorCommand::Restart

**Files modified:** `src/supervisor.rs`

**What was done:**
- Removed `#[allow(dead_code)]` from `SupervisorCommand::Restart` variant.
- Removed `#[allow(dead_code)]` from `restart_server` function.
- Updated doc comments on both to describe Phase 1 usage (foreground stdin command
  reader) instead of the previous "Phase 3 only" comment.

**Commit:** `feat(01-04): remove #[allow(dead_code)] from restart_server and SupervisorCommand::Restart`

---

### Task 3 — Add integration tests for stdin commands

**Files modified:** `tests/cli_integration_test.rs`

**What was done:**
Added three new integration tests using a new `spawn_hub_collecting` helper:

- `test_stdin_restart_command`: Starts hub with two `sleep 300` servers, waits 5s,
  sends `restart server-a`, waits 5s for restart + reprinted table, sends `status`,
  kills child, asserts stdout contains `server-a`, `server-b`, and `running`.
- `test_stdin_restart_unknown_server`: Starts hub, sends `restart nonexistent`, asserts
  stderr contains `not found`.
- `test_stdin_help_command`: Starts hub, sends `help`, asserts stderr contains
  `Available commands`, `restart`, and `status`.

**Implementation strategy:** Background drain threads (detached, non-scoped) accumulate
stdout/stderr into `Arc<Mutex<String>>` buffers. Tests use `std::thread::sleep` for
timing, then kill the child (closing pipes so drain threads exit), then assert buffer
contents. This avoids the blocking `thread::scope` deadlock where `read_line` on an
open pipe never returns EOF.

**Commit:** `feat(01-04): add integration tests for stdin restart, unknown server, and help commands`

---

### Task 4 — Final validation

All validation checks passed:

```
cargo fmt -- --check     ✓  (exit 0)
cargo clippy -- -D warnings  ✓  (exit 0, no warnings)
cargo test --test cli_integration_test  ✓  (13/13 tests pass)
grep -r "unwrap()" src/   ✓  (no results)
grep -r "allow(dead_code)" src/supervisor.rs  ✓  (no results)
```

---

## Acceptance Criteria Verification

| Criterion | Status |
|---|---|
| `src/main.rs` contains `run_foreground_loop` | PASS |
| `src/main.rs` contains `handle_stdin_command` | PASS |
| `handle_stdin_command` contains `strip_prefix("restart ")` | PASS |
| `handle_stdin_command` contains `supervisor::restart_server` | PASS |
| `handle_stdin_command` contains `output::print_status_table` | PASS |
| `handle_stdin_command` matches on `"status"` and `"help"` | PASS |
| `run_foreground_loop` uses `tokio::select!` with stdin + signal branches | PASS |
| `run_foreground_loop` handles `Ok(None)` by falling back to signal wait | PASS |
| `Commands::Start` calls `run_foreground_loop` | PASS |
| `src/supervisor.rs` has NO `#[allow(dead_code)]` on `restart_server` | PASS |
| `src/supervisor.rs` has NO `#[allow(dead_code)]` on `SupervisorCommand::Restart` | PASS |
| `tests/cli_integration_test.rs` contains `test_stdin_restart_command` | PASS |
| `test_stdin_restart_command` writes `restart server-a\n` to stdin | PASS |
| `test_stdin_restart_command` asserts stdout contains "server-a" and "running" | PASS |
| `tests/cli_integration_test.rs` contains `test_stdin_restart_unknown_server` | PASS |
| `test_stdin_restart_unknown_server` asserts stderr contains "not found" | PASS |
| `tests/cli_integration_test.rs` contains `test_stdin_help_command` | PASS |
| `test_stdin_help_command` asserts stderr contains "Available commands" | PASS |
| `cargo fmt -- --check` exits 0 | PASS |
| `cargo clippy -- -D warnings` exits 0 | PASS |
| `cargo test` exits 0 (all 13 tests pass) | PASS |
| No `unwrap()` in any `src/` file | PASS |

---

## Gap Closed

**GAP-01 / PROC-03**: "User can restart a specific server by name" is now fully satisfied.
The user can type `restart server-a` while the hub is running in the terminal to restart
only that server. The remaining servers continue running uninterrupted. After the restart,
a fresh status table is printed showing the new state and PID.

The `mcp-hub restart <name>` CLI subcommand still exits 1 with a message (daemon IPC is
not available in Phase 1). The gap closure is that restart is now fully functional in
the foreground interactive mode.

---

## Files Changed

| File | Change |
|---|---|
| `src/main.rs` | Added `run_foreground_loop`, `handle_stdin_command`; replaced bare signal wait |
| `src/supervisor.rs` | Removed `#[allow(dead_code)]` from `restart_server` and `SupervisorCommand::Restart`; updated doc comments |
| `tests/cli_integration_test.rs` | Added `spawn_hub_collecting` helper; added 3 new stdin integration tests |

---

*Executed: 2026-04-02*
