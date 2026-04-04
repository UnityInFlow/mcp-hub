---
plan_id: "03-03"
title: "Daemon Mode + Socket IPC"
status: "complete"
---

# Plan 03-03 Execution Summary

## What was implemented

### Task 1: `--daemon` flag added to CLI
- `src/cli.rs`: `Commands::Start` changed from a unit variant to a struct variant with `daemon: bool` field
- `src/main.rs`: All `Commands::Start` match arms updated to destructure `{ daemon }`

### Task 2: `src/daemon.rs` created
- `socket_path()` — resolves `~/.config/mcp-hub/mcp-hub.sock`, creates parent dir
- `pid_path()` — resolves `~/.config/mcp-hub/mcp-hub.pid`
- `write_pid_file()` / `remove_pid_file()` — PID file lifecycle management
- `check_existing_daemon()` — synchronous `UnixStream::connect` before fork; returns error if live daemon found, removes stale files if PID is dead
- `cleanup_stale_files()` — uses `kill(pid, 0)` / `ESRCH` to distinguish dead vs alive
- `daemonize_process()` — manual double-fork + setsid + `/dev/null` redirect (works on macOS; `nix::unistd::daemon()` is Linux/FreeBSD only)
- `redirect_to_dev_null()` — uses `nix::libc::dup2` to redirect fd 0/1/2

### Task 3: `src/control.rs` created
- `DaemonRequest` — serde-tagged enum: `Status | Stop | Restart { name } | Logs { server, lines } | Reload`
- `DaemonResponse` — `{ ok, data?, error? }` with `success()`, `ok_empty()`, `err()` constructors
- `DaemonState` — shared state for connection handlers: handles, log_agg, shutdown token, color flag
- `run_control_socket()` — binds UnixListener, accepts connections in a loop, shuts down when token cancelled, removes socket on exit
- `handle_connection()` — one request-response per connection (newline-delimited JSON)
- `dispatch_request()` — routes Status/Stop/Restart/Logs/Reload to appropriate handlers
- `send_daemon_command()` — client-side helper with timeout for CLI commands

### Task 4: `src/main.rs` restructured
- `#[tokio::main]` removed — replaced with synchronous `fn main()`
- Pre-fork work in `fn main()`: `check_existing_daemon` → `daemonize_process` → `write_pid_file`
- Tokio runtime built with `Builder::new_multi_thread().enable_all().build()?.block_on(async_main(cli))`
- `async fn async_main()` contains all async logic
- Daemon mode path: spawns control socket task, waits for SIGTERM or Stop command, stops servers, removes PID file
- Foreground mode path: unchanged behavior from Phase 2
- `Commands::Stop/Restart/Status/Logs` now route to daemon via `send_daemon_command`

### Task 5: `tests/daemon.rs` created (10 tests)
- `socket_path_returns_valid` — path ends with `mcp-hub.sock`, parent dir exists
- `pid_path_returns_valid` — path ends with `mcp-hub.pid`
- `write_and_read_pid_file` — writes and reads back current PID
- `remove_pid_file_deletes_file` — file removed after call
- `remove_pid_file_is_idempotent` — no panic on non-existent file
- `check_existing_daemon_with_no_socket` — returns Ok when no socket exists (Unix)
- `daemon_request_serialization` — round-trip all 5 variants, asserts JSON shapes
- `daemon_response_constructors` — verifies ok/data/error fields for all 3 constructors
- `daemon_response_serialization` — skip_serializing_if works correctly
- `control_socket_round_trip` — full integration: bind socket, Status request, Stop request, task exits

Updated `tests/cli_integration_test.rs` and `tests/logs_command.rs` to match new Phase 3 behavior (CLI commands now connect to daemon socket, error messages changed).

## Key design decisions

**Double-fork instead of `nix::unistd::daemon()`**: The nix crate's `daemon()` function is gated to Linux/FreeBSD/Solaris and is not available on macOS. Implemented the equivalent manually using `fork()` + `setsid()` + second `fork()` + `/dev/null` redirect, which works on all Unix platforms including macOS.

**`Arc::try_unwrap` pattern for handles**: After `drop(daemon_state)`, the `handles_arc` clone is the only remaining reference, allowing `try_unwrap` to succeed cleanly.

**`color` field suppressed**: `DaemonState::color` is reserved for future log colorization and annotated with `#[allow(dead_code)]`.

## Acceptance criteria verification

- `daemon: bool` in src/cli.rs: PASS
- `#[arg(long)]` before daemon: PASS
- `Commands::Start { daemon }` in src/main.rs: PASS
- All daemon.rs functions present: PASS
- `nix::unistd` used (setsid, fork, ForkResult): PASS
- `fn cleanup_stale_files` present: PASS
- `mcp-hub.sock` and `mcp-hub.pid` paths: PASS
- `mod daemon` in main.rs: PASS
- All control.rs types and functions present: PASS
- `serde(tag = "cmd"` in control.rs: PASS
- `mod control` in main.rs: PASS
- Synchronous `fn main() -> anyhow::Result`: PASS
- `async fn async_main`: PASS
- `tokio::runtime::Builder::new_multi_thread`: PASS
- All daemon:: calls in main.rs: PASS
- `control::run_control_socket` in main.rs: PASS
- `DaemonState` in main.rs: PASS
- No `#[tokio::main]` in main.rs: PASS
- All daemon tests pass: PASS (10/10)
- All existing tests pass: PASS (120/120 total)
- No `unwrap()` in production code: PASS
- `cargo clippy -D warnings`: PASS

## Commits

- `feat(03-03-01): add --daemon flag to Commands::Start`
- `feat(03-03-02): create daemon module with PID file, socket paths, and daemonize`
- `feat(03-03-03): create control socket module with DaemonRequest/Response IPC types`
- `feat(03-03-04): restructure main() for fork-before-runtime and daemon startup path`
- `feat(03-03-05): add daemon mode tests and update CLI integration tests for Phase 3 behavior`
