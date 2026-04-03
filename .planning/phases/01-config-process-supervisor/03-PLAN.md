---
plan_id: "01-03"
title: "CLI integration + output formatting + integration tests"
phase: 1
wave: 3
depends_on:
  - "01-01"
  - "01-02"
files_modified:
  - src/main.rs
  - src/output.rs
  - src/cli.rs
  - tests/cli_integration_test.rs
requirements_addressed:
  - CFG-01
  - CFG-02
  - PROC-01
  - PROC-02
  - PROC-03
  - PROC-07
  - DMN-01
autonomous: true
---

# Plan 03: CLI Integration + Output Formatting + Integration Tests

<objective>
Wire together the config loader, process supervisor, and CLI into a working `mcp-hub` binary. Implement the colored status table output, the full subcommand dispatch (start, stop, restart), tracing/verbosity configuration, and comprehensive integration tests that exercise the real binary with assert_cmd. After this plan, `mcp-hub start` reads a TOML config, launches all servers, prints a status table, and shuts down cleanly on Ctrl+C.
</objective>

---

## Task 1: Implement output formatting with colored status table

<task id="01-03-T1">
<read_first>
- src/types.rs (ProcessState enum and its Display impl)
- src/supervisor.rs (ServerHandle struct — state_rx is watch::Receiver<(ProcessState, Option<u32>)>)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 7: Terminal Output — comfy-table, owo-colors code patterns, use_colors function, print_status_table function)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-15: status table after launch, D-16: colors, D-17: quiet by default)
</read_first>

<action>
Create `src/output.rs` with the following:

1. `pub fn use_colors(no_color_flag: bool) -> bool`:
   - Return `false` if `no_color_flag` is true
   - Return `false` if `std::io::stdout().is_terminal()` returns false (piped output)
   - Return `true` otherwise
   - Import `std::io::IsTerminal`

2. `pub fn print_status_table(servers: &[(String, ProcessState, Option<u32>)], color: bool)`:
   - Create a `comfy_table::Table`
   - Set header row: `["Name", "State", "PID"]`
   - For each server tuple:
     - Name cell: plain text
     - State cell: if `color` is true, apply colors:
       - `Running` -> green
       - `Starting` -> yellow
       - `Backoff { .. }` -> yellow
       - `Stopping` -> yellow
       - `Fatal` -> red
       - `Stopped` -> dark grey
     - PID cell: display the u32, or "-" if None
   - Print the table to stdout with `println!("{table}")`

3. `pub fn collect_states_from_handles(handles: &[crate::supervisor::ServerHandle]) -> Vec<(String, ProcessState, Option<u32>)>`:
   - For each handle, borrow the current state from `handle.state_rx.borrow()`
   - Collect into a Vec of (name, state, pid) tuples
   - This is a non-async, non-blocking operation (watch::Receiver::borrow is instant)

4. `pub fn configure_tracing(verbose: u8)`:
   - `0` (default): set filter to `warn` — quiet by default (D-17)
   - `1` (`-v`): set filter to `info` — show start/stop events
   - `2+` (`-vv`): set filter to `debug` — show spawn details
   - Initialize `tracing_subscriber::fmt()` with the computed `EnvFilter`
   - Use `tracing_subscriber::fmt().with_env_filter(filter).init()`
</action>

<acceptance_criteria>
- File `src/output.rs` exists
- `src/output.rs` contains `pub fn use_colors(no_color_flag: bool) -> bool`
- `use_colors` checks `std::io::stdout().is_terminal()`
- `src/output.rs` contains `pub fn print_status_table`
- `print_status_table` creates a `comfy_table::Table` with header `["Name", "State", "PID"]`
- `print_status_table` applies green color for `Running` state when color is true
- `print_status_table` applies red color for `Fatal` state when color is true
- `src/output.rs` contains `pub fn collect_states_from_handles`
- `src/output.rs` contains `pub fn configure_tracing`
- `configure_tracing` maps verbose=0 to `warn`, verbose=1 to `info`, verbose>=2 to `debug`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 2: Wire CLI subcommand dispatch in main.rs

<task id="01-03-T2">
<read_first>
- src/main.rs (current minimal stub)
- src/cli.rs (Cli, Commands, RestartArgs)
- src/config.rs (find_and_load_config)
- src/supervisor.rs (start_all_servers, wait_for_initial_states, stop_all_servers, restart_server, ServerHandle)
- src/output.rs (use_colors, print_status_table, collect_states_from_handles, configure_tracing)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 4: run() function pattern, Section 6: exit codes)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-10: Ctrl+C handler, D-15: wait then print status)
</read_first>

<action>
Rewrite `src/main.rs` to wire everything together:

```rust
mod cli;
mod config;
mod output;
mod supervisor;
mod types;

use anyhow::Context;
use clap::Parser;
use cli::{Cli, Commands};
use tokio_util::sync::CancellationToken;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    
    output::configure_tracing(cli.verbose);
    let color = output::use_colors(cli.no_color);
    
    match cli.command {
        Commands::Start => {
            let config = config::find_and_load_config(
                cli.config.as_deref()
            ).context("Failed to load config")?;
            
            if config.servers.is_empty() {
                anyhow::bail!("No servers defined in config. Add servers to mcp-hub.toml or run `mcp-hub init`.");
            }
            
            let shutdown = CancellationToken::new();
            let mut handles = supervisor::start_all_servers(&config, shutdown.clone()).await;
            
            // Wait for servers to reach initial state (Running, Backoff, or Fatal)
            supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;
            
            // Print status table (D-15)
            let states = output::collect_states_from_handles(&handles);
            output::print_status_table(&states, color);
            
            // Block on Ctrl+C or SIGTERM (DMN-01)
            wait_for_shutdown_signal().await?;
            
            tracing::info!("Shutting down all servers...");
            shutdown.cancel();
            supervisor::stop_all_servers(handles).await;
            
            tracing::info!("All servers stopped.");
            Ok(())
        }
        Commands::Stop => {
            // Phase 1: foreground mode only — stop is Ctrl+C.
            // Daemon mode (Phase 3) will implement socket-based stop.
            eprintln!("mcp-hub stop: no daemon running (foreground mode uses Ctrl+C to stop)");
            std::process::exit(1);
        }
        Commands::Restart(args) => {
            // Phase 1: restart requires the hub to be running in foreground.
            // For now, this prints a message. Full implementation with daemon IPC is Phase 3.
            // However, the restart_server function IS implemented and wired into the supervisor.
            eprintln!(
                "mcp-hub restart {}: restart is available during foreground operation via the supervisor. \
                 Daemon-mode restart will be available in a future version.",
                args.name
            );
            std::process::exit(1);
        }
    }
}

async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())
            .context("Failed to install SIGTERM handler")?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl+C received");
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
            .context("Failed to install Ctrl+C handler")?;
        tracing::info!("Ctrl+C received");
    }
    Ok(())
}
```

Key behaviors:
- `mcp-hub start`: loads config, spawns all servers, waits for initial states, prints status table, blocks on Ctrl+C/SIGTERM, then shuts down gracefully. Exit 0.
- `mcp-hub stop`: prints message about foreground mode, exits 1 (daemon mode is Phase 3).
- `mcp-hub restart <name>`: prints message about foreground mode, exits 1 (daemon mode is Phase 3). The `restart_server` function is implemented in supervisor.rs and exercised during foreground operation when wired in Phase 3.
- Config validation errors: `find_and_load_config` returns anyhow::Error which propagates to main and prints to stderr, exit 1 (default anyhow behavior).
- Empty config (0 servers): explicit `bail!` with actionable message.

Update `src/main.rs` module declarations to include `mod output;`.
</action>

<acceptance_criteria>
- `src/main.rs` contains `mod output;` and `mod supervisor;` and `mod config;` and `mod cli;` and `mod types;`
- `src/main.rs` contains `output::configure_tracing(cli.verbose)`
- `src/main.rs` contains `output::use_colors(cli.no_color)`
- `src/main.rs` contains `config::find_and_load_config`
- `src/main.rs` contains `supervisor::start_all_servers`
- `src/main.rs` contains `supervisor::wait_for_initial_states`
- `src/main.rs` contains `output::print_status_table`
- `src/main.rs` contains `wait_for_shutdown_signal` function
- `wait_for_shutdown_signal` handles both `ctrl_c` and `SIGTERM` on Unix
- `src/main.rs` contains `shutdown.cancel()` followed by `stop_all_servers`
- Commands::Stop branch prints a message and calls `std::process::exit(1)`
- `src/main.rs` does NOT contain `unwrap()`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 3: Write CLI integration tests

<task id="01-03-T3">
<read_first>
- src/main.rs (full CLI dispatch — verify the binary behavior)
- src/config.rs (error messages for validation failures)
- tests/fixtures/valid.toml (test fixture for successful start)
- tests/fixtures/invalid-missing-command.toml (test fixture for validation error)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 10: integration tests table)
- .planning/phases/01-config-process-supervisor/01-VALIDATION.md (task 01-05, 01-06)
</read_first>

<action>
Create `tests/cli_integration_test.rs` with integration tests using `assert_cmd`:

1. `test_version_flag`:
   - Run `mcp-hub --version`
   - Assert exit code 0
   - Assert stdout contains "mcp-hub"

2. `test_help_flag`:
   - Run `mcp-hub --help`
   - Assert exit code 0
   - Assert stdout contains "PM2 for MCP servers"

3. `test_start_help`:
   - Run `mcp-hub start --help`
   - Assert exit code 0
   - Assert stdout contains "Start all configured MCP servers"

4. `test_missing_config_exits_nonzero`:
   - Run `mcp-hub start --config /nonexistent/path/config.toml`
   - Assert exit code is non-zero (1)
   - Assert stderr contains "Failed to" or "config" (error message about missing file)

5. `test_invalid_toml_exits_nonzero`:
   - Create a tempfile with invalid TOML content: `this is not valid toml }{`
   - Run `mcp-hub start --config <tempfile>`
   - Assert exit code is non-zero
   - Assert stderr contains "TOML" or "Invalid" or "parse"

6. `test_empty_config_exits_nonzero`:
   - Create a tempfile with valid but empty TOML: `# empty config`
   - Run `mcp-hub start --config <tempfile>`
   - Assert exit code is non-zero
   - Assert stderr contains "No servers defined" or "init"

7. `test_bad_command_in_config_starts_and_shows_fatal`:
   - Create a tempfile with TOML:
     ```toml
     [servers.broken]
     command = "/nonexistent/binary/that/does/not/exist"
     ```
   - Run `mcp-hub start --config <tempfile>` — the binary will start, try to spawn, fail, and mark server as Fatal in the status table (or fail on spawn error)
   - This test needs special handling: either assert the status table contains "fatal" or assert a spawn error is printed. Use a timeout on the assert_cmd invocation since the binary blocks on Ctrl+C.
   - Use `assert_cmd::Command::new(...).timeout(std::time::Duration::from_secs(15))` to prevent hanging.

8. `test_start_with_valid_config_shows_status_table`:
   - Create a tempfile with TOML:
     ```toml
     [servers.sleeper]
     command = "sleep"
     args = ["300"]
     ```
   - Run `mcp-hub start --config <tempfile>` with a short timeout (5 seconds)
   - The binary should start, print a status table with "sleeper" and "running", then we kill it via the timeout
   - Assert stdout contains "sleeper"
   - Assert stdout contains "running" (the state in the status table)

9. `test_no_color_flag`:
   - Run `mcp-hub start --no-color --config <valid-config-tempfile>` with a short timeout
   - Assert stdout does NOT contain ANSI escape codes (no `\x1b[` sequences)
   - Assert stdout contains "sleeper" (server name is present, just without color)

10. `test_stop_without_daemon_prints_message`:
    - Run `mcp-hub stop`
    - Assert exit code is non-zero
    - Assert stderr contains "foreground" or "Ctrl+C"

All tests use `assert_cmd::Command::cargo_bin("mcp-hub")` and `tempfile::NamedTempFile` for config files. Tests that start the binary with a valid config must use `.timeout()` to prevent blocking indefinitely.
</action>

<acceptance_criteria>
- File `tests/cli_integration_test.rs` exists
- `tests/cli_integration_test.rs` contains `test_version_flag`
- `tests/cli_integration_test.rs` contains `test_help_flag`
- `tests/cli_integration_test.rs` contains `test_missing_config_exits_nonzero`
- `tests/cli_integration_test.rs` contains `test_invalid_toml_exits_nonzero`
- `tests/cli_integration_test.rs` contains `test_empty_config_exits_nonzero`
- `tests/cli_integration_test.rs` contains `test_start_with_valid_config_shows_status_table`
- `tests/cli_integration_test.rs` contains `test_no_color_flag`
- `tests/cli_integration_test.rs` contains `test_stop_without_daemon_prints_message`
- `tests/cli_integration_test.rs` contains `test_bad_command_in_config_starts_and_shows_fatal`
- `tests/cli_integration_test.rs` contains `test_start_help`
- Tests use `assert_cmd::Command::cargo_bin("mcp-hub")`
- Tests with valid configs use `.timeout()` to prevent hanging
- `cargo test --test cli_integration_test` exits 0
</acceptance_criteria>
</task>

---

## Task 4: Final validation — all tests, clippy, fmt

<task id="01-03-T4">
<read_first>
- src/main.rs (full implementation)
- src/cli.rs (CLI definitions)
- src/config.rs (config loading)
- src/types.rs (domain types)
- src/supervisor.rs (process management)
- src/output.rs (terminal output)
- tests/config_test.rs (config unit tests)
- tests/supervisor_test.rs (supervisor unit tests)
- tests/cli_integration_test.rs (CLI integration tests)
- .planning/phases/01-config-process-supervisor/01-VALIDATION.md (full verification map)
</read_first>

<action>
Run the complete validation suite:

1. `cargo fmt` — format all code
2. `cargo clippy -- -D warnings` — fix any clippy warnings (common issues: unused imports, unnecessary clones, missing error handling)
3. `cargo test` — run ALL tests (unit + integration)
4. Verify no `unwrap()` in any `src/` file: `grep -r "unwrap()" src/` should return nothing
5. Verify no `any` or placeholder types that bypass safety

Fix any issues found. Common Phase 1 fixes:
- Remove unused `use` statements
- Replace `clone()` calls flagged by clippy with references
- Add `#[allow(dead_code)]` ONLY for functions that are intentionally unused in Phase 1 but needed in Phase 2+ (e.g., `restart_server` may not be called from main.rs yet if restart is deferred to daemon mode)
- Ensure all match arms in `ProcessState` display are exhaustive

After all fixes, run the full suite one final time:
```bash
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test
```
All three must exit 0.
</action>

<acceptance_criteria>
- `cargo fmt -- --check` exits 0 (code is formatted)
- `cargo clippy -- -D warnings` exits 0 (no clippy warnings)
- `cargo test` exits 0 (all tests pass — config, supervisor, and CLI integration)
- `grep -r "unwrap()" src/` returns no results (no unwrap in production code)
- No `#[allow(clippy::` directives added to silence legitimate warnings
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run these commands after all tasks are complete:

```bash
cargo fmt -- --check              # Must exit 0
cargo clippy -- -D warnings       # Must exit 0
cargo test                        # Must exit 0 — ALL tests pass
cargo test --test config_test     # Must exit 0 — config tests pass
cargo test --test supervisor_test # Must exit 0 — supervisor tests pass  
cargo test --test cli_integration_test  # Must exit 0 — integration tests pass
grep -r "unwrap()" src/           # Must return empty (no unwrap in production code)
```

Manual smoke test (optional, not automated):
```bash
# Create a test config
cat > /tmp/test-mcp-hub.toml << 'EOF'
[servers.test-sleep]
command = "sleep"
args = ["300"]

[servers.test-echo]
command = "bash"
args = ["-c", "echo started >&2 && sleep 300"]
EOF

# Start the hub — should print status table with both servers "running"
cargo run -- start --config /tmp/test-mcp-hub.toml

# Press Ctrl+C — should see "Shutting down" message, both servers stop, exit 0
```

### must_haves
- [ ] `mcp-hub start --config <path>` reads TOML, spawns all servers, prints colored status table, blocks on Ctrl+C (CFG-01, CFG-02, PROC-01, DMN-01)
- [ ] Status table shows Name, State (colored), and PID columns (D-15, D-16)
- [ ] `--no-color` flag disables ANSI colors; piped output also disables colors (D-16)
- [ ] `-v` sets tracing to info level; `-vv` sets to debug; default is warn (D-17)
- [ ] `mcp-hub stop` prints foreground-mode message and exits 1 (PROC-02 — full implementation in Phase 3 daemon mode)
- [ ] `mcp-hub restart <name>` prints foreground-mode message and exits 1 (PROC-03 — restart_server function exists in supervisor.rs for Phase 3 wiring)
- [ ] Missing/invalid config: clear error on stderr, non-zero exit (CFG-02)
- [ ] Empty config (no servers): clear error message mentioning `mcp-hub init` (CFG-02)
- [ ] Ctrl+C triggers graceful SIGTERM->5s->SIGKILL shutdown of all children (PROC-07, DMN-01)
- [ ] SIGTERM (Unix) also triggers graceful shutdown (DMN-01)
- [ ] All config unit tests pass (7 tests from Plan 01)
- [ ] All supervisor unit tests pass (7 tests from Plan 02)
- [ ] All CLI integration tests pass (8-10 tests from this plan)
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] No `unwrap()` in any src/ file
</verification>
