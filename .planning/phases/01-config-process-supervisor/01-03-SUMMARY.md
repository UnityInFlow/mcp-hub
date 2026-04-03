# Plan 01-03 Summary: CLI Integration + Output Formatting + Integration Tests

**Executed:** 2026-04-02
**Status:** Complete — all 4 tasks done, all tests pass

---

## What Was Done

### Task 1 — src/output.rs (new file)

Created `src/output.rs` with four public functions:

- `use_colors(no_color_flag: bool) -> bool` — returns false if `--no-color` flag is set or stdout is not a TTY (`std::io::IsTerminal`)
- `print_status_table(servers, color)` — renders a `comfy_table::Table` with headers `["Name", "State", "PID"]`; applies green/yellow/red/dark-grey colors via `comfy_table::Color` when color is enabled
- `collect_states_from_handles(handles) -> Vec<(String, ProcessState, Option<u32>)>` — non-blocking snapshot of all server states via `watch::Receiver::borrow`
- `configure_tracing(verbose: u8)` — sets `tracing_subscriber` filter: 0=warn, 1=info, 2+=debug

Also added `pub mod output;` to `src/lib.rs`.

### Task 2 — src/main.rs (rewrite)

Rewrote `main.rs` to wire the full CLI dispatch:

- `Commands::Start`: loads config → validates (bails with actionable message if empty) → spawns all servers via `start_all_servers` → waits for initial states → prints colored status table → blocks on Ctrl+C/SIGTERM → cancels shutdown token → `stop_all_servers`
- `Commands::Stop`: prints foreground-mode message, exits 1 (daemon IPC is Phase 3)
- `Commands::Restart(args)`: prints foreground-mode message, exits 1 (daemon IPC is Phase 3)
- `wait_for_shutdown_signal()`: on Unix handles both SIGINT (Ctrl+C) and SIGTERM via `tokio::select!`; on non-Unix handles Ctrl+C only

Also added `#[allow(dead_code)]` annotations to `supervisor.rs` for `SupervisorCommand::Restart` and `restart_server()` — these are Phase 3 APIs intentionally unused in Phase 1 foreground mode.

### Task 3 — tests/cli_integration_test.rs (new file)

Created 10 integration tests using `assert_cmd::Command::cargo_bin("mcp-hub")`:

| Test | What it verifies |
|---|---|
| `test_version_flag` | `--version` exits 0, stdout contains "mcp-hub" |
| `test_help_flag` | `--help` exits 0, stdout contains "PM2 for MCP servers" |
| `test_start_help` | `start --help` exits 0, stdout contains "Start all configured MCP servers" |
| `test_missing_config_exits_nonzero` | nonexistent config path → non-zero exit, stderr contains error |
| `test_invalid_toml_exits_nonzero` | invalid TOML → non-zero exit, stderr mentions parse error |
| `test_empty_config_exits_nonzero` | empty config (no servers) → non-zero exit, stderr mentions "No servers defined" |
| `test_stop_without_daemon_prints_message` | `stop` → exits 1, stderr mentions "foreground" or "Ctrl+C" |
| `test_bad_command_in_config_starts_and_shows_fatal` | nonexistent binary → server goes Fatal, status table shows server name (15s timeout) |
| `test_start_with_valid_config_shows_status_table` | `sleep 300` server → status table shows "sleeper" and "running" (5s timeout) |
| `test_no_color_flag` | `--no-color` → no ANSI `\x1b[` escape codes in stdout (5s timeout) |

### Task 4 — Final validation

Fixed a doc test failure: the state machine diagram in `supervisor.rs` docstring was in a plain ` ``` ` block (parsed as Rust), changed to ` ```text ` to prevent the doc test runner from trying to compile it as code.

Final validation results:
- `cargo fmt -- --check`: 0 issues
- `cargo clippy -- -D warnings`: 0 issues
- `cargo test`: 24 tests pass (6 suites)
  - lib doc tests: 0 (none)
  - supervisor doc tests: 0 (text blocks, not compiled)
  - `config_test`: 7 tests pass
  - `supervisor_test`: 7 tests pass
  - `cli_integration_test`: 10 tests pass
- `grep -r "unwrap()" src/`: empty (no unwrap in production code)

---

## Files Modified

| File | Change |
|---|---|
| `src/output.rs` | Created — colored status table, tracing setup, state collection |
| `src/lib.rs` | Added `pub mod output;` |
| `src/main.rs` | Rewrote — full CLI dispatch, structured shutdown |
| `src/supervisor.rs` | Added `#[allow(dead_code)]` to Phase 3 APIs; fixed doc block type |
| `tests/cli_integration_test.rs` | Created — 10 integration tests |

---

## Requirements Addressed

- **CFG-01, CFG-02**: `start` reads and validates TOML config; missing/invalid config → clear stderr error, non-zero exit
- **PROC-01**: `start` spawns all servers and prints status table
- **PROC-02**: `stop` stub prints foreground-mode message (daemon mode is Phase 3)
- **PROC-03**: `restart` stub prints foreground-mode message (daemon mode is Phase 3)
- **PROC-07**: Ctrl+C → `shutdown.cancel()` → SIGTERM+5s+SIGKILL via existing `stop_all_servers`
- **DMN-01**: SIGTERM also triggers graceful shutdown (Unix only, via `tokio::select!`)

---

## Commits

1. `feat(01-03): add output formatting — colored status table and tracing setup`
2. `feat(01-03): wire CLI dispatch in main.rs — start/stop/restart subcommands`
3. `feat(01-03): add CLI integration tests — 10 tests covering start/stop/errors/status-table`
4. `fix(01-03): mark state machine diagram as text block to prevent doc test failures`
