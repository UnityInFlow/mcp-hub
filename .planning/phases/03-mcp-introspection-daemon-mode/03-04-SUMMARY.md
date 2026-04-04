# Plan 03-04 Summary: CLI Wiring + Config Reload + Integration Tests

## Status: COMPLETE

All 6 tasks executed and committed. All 132 tests pass, cargo clippy clean, cargo fmt clean.

---

## Tasks Completed

### Task 1: PartialEq + Eq on ServerConfig and HubConfig
- Added `PartialEq, Eq` derives to `ServerConfig` in `src/config.rs`
- Added `PartialEq` to `HubConfig` in `src/config.rs`
- All fields (`String`, `Vec<String>`, `HashMap<String, String>`, `Option<*>`) support these traits; no float fields present

**Commit:** `feat(03-04-01): derive PartialEq + Eq on ServerConfig and PartialEq on HubConfig`

---

### Task 2: Config diff and apply logic
Three new public functions added to `src/supervisor.rs`:

- **`start_single_server`** — extracted from `start_all_servers` loop body; spawns one supervisor task
- **`stop_named_server`** — removes a handle by name, sends `Shutdown`, awaits the task
- **`apply_config_diff`** — diffs `old_config` vs `new_config` using `HashSet` set operations:
  - Removed servers (old − new): stopped via `stop_named_server`
  - New servers (new − old): started via `start_single_server`
  - Changed servers (intersection, config differs): stopped then re-started
  - Unchanged servers (intersection, config equal): left running (no restart)
  - Returns `(added, removed, changed)` counts

`start_all_servers` refactored to delegate to `start_single_server`.

**Commit:** `feat(03-04-02): add start_single_server, stop_named_server, apply_config_diff to supervisor`

---

### Task 3: SIGHUP handler and Reload command
- **`src/cli.rs`**: Added `Commands::Reload` subcommand
- **`src/control.rs`**: `DaemonRequest::Reload` wired — sends `SIGHUP` to self via `nix::sys::signal::kill`, triggering the main event loop's SIGHUP handler
- **`src/main.rs`**:
  - Daemon mode event loop replaced with a `select!` loop handling `sighup.recv()`, `sigterm.recv()`, `ctrl_c()`, and `shutdown.cancelled()`
  - `handle_reload()` async helper added — loads new config, calls `apply_config_diff`, updates `current_config`
  - `current_config` tracked as mutable state so diffs are always accurate
  - Foreground mode loop also has `sighup.recv()` arm (logs notice; full reload requires daemon mode)
  - Non-Unix fallback uses `ctrl_c()` + `shutdown.cancelled()`

**Commit:** `feat(03-04-03): add Reload CLI command, SIGHUP handler, handle_reload, wire DaemonRequest::Reload`

---

### Task 4: Wire all CLI commands to daemon socket
Updated `src/main.rs` to use `send_daemon_command` for all non-Start commands:

| Command | Request | Timeout | Success message |
|---|---|---|---|
| `stop` | `DaemonRequest::Stop` | 5s | "Daemon stop command sent. Shutting down..." |
| `restart <name>` | `DaemonRequest::Restart { name }` | 10s | "Restart signal sent to '<name>'." |
| `status` | `DaemonRequest::Status` | 5s | Pretty-printed JSON |
| `logs` | `DaemonRequest::Logs` | 5s | One line per log entry |
| `reload` | `DaemonRequest::Reload` | 5s | "Reload signal sent to daemon." |

All error paths use `eprintln!` + `std::process::exit(1)`. No "Phase 3 will implement" stubs remain. The `send_daemon_command` function already provides a clear "Is the daemon running? Start with: mcp-hub start --daemon" message on connection failure.

**Commit:** `feat(03-04-04): wire all CLI commands to daemon socket with correct messages`

---

### Task 5: Config reload tests
Created `tests/config_reload.rs` with 6 tests:

1. **`server_config_partial_eq`** — synchronous; verifies equality and inequality across all fields
2. **`unchanged_config_no_restarts`** — identical configs produce `(0, 0, 0)` and handle count stays 1
3. **`add_new_server`** — old config ["a"], new config ["a", "b"] → `(1, 0, 0)`, 2 handles
4. **`remove_server`** — old ["a", "b"], new ["a"] → `(0, 1, 0)`, 1 handle, only "a" remains
5. **`change_server_command`** — same name, different args → `(0, 0, 1)`, task ID changes
6. **`mixed_config_diff`** — old ["a", "b", "c"], new ["a", "c_modified", "d"] → `(1, 1, 1)`, 3 handles

All tests use `sleep 9999` as a harmless long-running process and cancel via `CancellationToken`.

**Commit:** `test(03-04-05): add config_reload integration tests covering all diff cases`

---

### Task 6: Daemon lifecycle integration tests
Two additions:

**`src/daemon.rs`**: Added `MCP_HUB_SOCKET` and `MCP_HUB_PID` environment variable overrides to `socket_path()` and `pid_path()`. This enables test isolation without modifying `dirs::config_dir()` (which uses macOS Frameworks and cannot be overridden via `HOME`/`XDG_CONFIG_HOME`).

**`tests/cli_daemon.rs`**: 6 Unix-only (`#[cfg(unix)]`) integration tests using `assert_cmd` + temp directories:

1. **`daemon_creates_socket_and_pid`** — start daemon, verify socket and PID files appear
2. **`duplicate_daemon_prevention`** — second `start --daemon` fails with "already running" in stderr
3. **`status_from_daemon`** — `mcp-hub status` returns JSON containing server name
4. **`stop_shuts_down_daemon`** — `mcp-hub stop` exits 0, socket file disappears
5. **`restart_via_daemon`** — `mcp-hub restart test-server` exits 0, stdout contains "Restart signal sent"
6. **`stale_socket_cleanup`** — force-kill daemon with `kill -9`, new daemon starts successfully after cleanup

Each test injects `MCP_HUB_SOCKET` and `MCP_HUB_PID` pointing to a `TempDir` so tests are fully isolated.

**Commit:** `test(03-04-06): add daemon lifecycle integration tests with MCP_HUB_SOCKET/PID isolation`

---

## Verification

```
cargo build     — clean
cargo clippy -- -D warnings   — no issues
cargo test      — 132 passed (18 suites)
cargo fmt -- --check   — clean
```

---

## Phase 3 Requirements Met

All 9 Phase 3 requirements are now satisfied:

| ID | Requirement | How |
|---|---|---|
| MCP-01 | Hub introspects each server on startup | `run_introspection` in supervisor after first Healthy |
| MCP-02 | JSON-RPC IDs correctly correlated | `PendingMap` + `IdAllocator` dispatcher pattern |
| MCP-03 | Introspection results in status table | `McpCapabilities` in `ServerSnapshot`, rendered in output |
| MCP-04 | Transport type explicit in config | `transport` field in `ServerConfig` with `default_transport()` |
| CFG-03 | Config reloadable via SIGHUP or `mcp-hub reload` | `handle_reload` + SIGHUP handler + `DaemonRequest::Reload` |
| DMN-02 | `mcp-hub start --daemon` daemonizes | Double-fork via `daemonize_process()` |
| DMN-03 | Daemon communicates via Unix domain socket | `run_control_socket` + `send_daemon_command` |
| DMN-04 | `mcp-hub stop` connects to socket | `Commands::Stop` sends `DaemonRequest::Stop` |
| DMN-05 | Duplicate daemon prevention | `check_existing_daemon` + stale socket cleanup |
