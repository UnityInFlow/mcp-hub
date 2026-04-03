---
plan_id: "01-04"
title: "GAP-01 closure: stdin restart command for foreground mode"
phase: 1
wave: 4
depends_on:
  - "01-03"
gap_closure: true
files_modified:
  - src/main.rs
  - src/supervisor.rs
  - tests/cli_integration_test.rs
requirements_addressed:
  - PROC-03
autonomous: true
---

# Plan 04: GAP-01 Closure — Stdin Restart Command for Foreground Mode

<objective>
Close GAP-01 (PROC-03: "User can restart a specific server by name") by adding a stdin command reader to the foreground hub loop. When the hub is running in the terminal, the user can type `restart <name>` to restart a specific server without stopping the others. This avoids the need for daemon IPC while fully satisfying the PROC-03 requirement in Phase 1.
</objective>

---

## Task 1: Add stdin command reader to the foreground hub loop

<task id="01-04-T1">
<read_first>
- src/main.rs (the `Commands::Start` branch — the foreground loop that blocks on `wait_for_shutdown_signal`)
- src/supervisor.rs (`restart_server` function, `ServerHandle` struct, `SupervisorCommand::Restart`)
- src/output.rs (`collect_states_from_handles`, `print_status_table` — to reprint status after restart)
- src/types.rs (ProcessState enum — to understand what states look like after restart)
</read_first>

<action>
Modify `src/main.rs` to replace the simple `wait_for_shutdown_signal().await` call in the `Commands::Start` branch with a loop that concurrently:

1. Reads lines from stdin (via `tokio::io::BufReader::new(tokio::io::stdin()).lines()`)
2. Waits for shutdown signals (Ctrl+C / SIGTERM)

When a line is read from stdin:
- Trim whitespace
- If the line starts with `restart `, extract the server name after the prefix
- Call `supervisor::restart_server(&handles, &name).await`
- On success: log the restart, wait 2 seconds for the server to reach a new state, then reprint the status table
- On error (server not found): print the error to stderr
- If the line is `status`, reprint the current status table
- If the line is `help`, print available commands
- Ignore empty lines and unrecognized commands (print a hint to type `help`)

When a shutdown signal arrives, break the loop and proceed to graceful shutdown (existing behavior).

Specific code changes in `src/main.rs`:

Replace the block after `print_status_table` (from `// Block on Ctrl+C or SIGTERM` through `wait_for_shutdown_signal().await?;`) with a new `run_foreground_loop` async function call that:

```rust
async fn run_foreground_loop(
    handles: &[supervisor::ServerHandle],
    color: bool,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    // Print a hint that commands are available
    eprintln!("Type 'help' for available commands, or press Ctrl+C to stop.");

    loop {
        tokio::select! {
            // Read a line from stdin
            line = lines.next_line() => {
                match line {
                    Ok(Some(input)) => {
                        let trimmed = input.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        handle_stdin_command(trimmed, handles, color).await;
                    }
                    Ok(None) => {
                        // stdin closed (e.g., piped input exhausted) — wait for signal
                        wait_for_shutdown_signal().await?;
                        break;
                    }
                    Err(err) => {
                        tracing::warn!("Error reading stdin: {err}");
                        wait_for_shutdown_signal().await?;
                        break;
                    }
                }
            }
            // Wait for shutdown signal
            result = wait_for_shutdown_signal() => {
                result?;
                break;
            }
        }
    }

    Ok(())
}
```

Add a helper function:

```rust
async fn handle_stdin_command(
    input: &str,
    handles: &[supervisor::ServerHandle],
    color: bool,
) {
    if let Some(name) = input.strip_prefix("restart ") {
        let name = name.trim();
        if name.is_empty() {
            eprintln!("Usage: restart <server-name>");
            return;
        }
        match supervisor::restart_server(handles, name).await {
            Ok(()) => {
                eprintln!("Restart signal sent to '{name}'. Waiting for new state...");
                // Give the supervisor time to stop and re-spawn
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let states = output::collect_states_from_handles(handles);
                output::print_status_table(&states, color);
            }
            Err(err) => {
                eprintln!("Error: {err}");
            }
        }
    } else if input == "status" {
        let states = output::collect_states_from_handles(handles);
        output::print_status_table(&states, color);
    } else if input == "help" {
        eprintln!("Available commands:");
        eprintln!("  restart <name>  — Restart the named server");
        eprintln!("  status          — Show current server status");
        eprintln!("  help            — Show this help message");
        eprintln!("  Ctrl+C          — Shut down all servers and exit");
    } else {
        eprintln!("Unknown command: '{input}'. Type 'help' for available commands.");
    }
}
```

Important implementation notes:
- `wait_for_shutdown_signal` must be refactored to NOT consume the signal handler on first call. Since it is now inside a `tokio::select!` loop, wrap the signal setup OUTSIDE the loop (create the sigterm listener once, pass it into the loop). The simplest approach: move signal setup into `run_foreground_loop` and use `tokio::select!` directly on `ctrl_c()` and `sigterm.recv()` alongside `lines.next_line()`.
- The `#[allow(dead_code)]` on `restart_server` and the `SupervisorCommand::Restart` variant can now be removed since they are actively used.
- The `Commands::Restart(args)` CLI branch stays as-is (exits 1 with a message) because `mcp-hub restart <name>` as a separate CLI invocation still cannot reach a running hub without IPC. The gap closure is that restart is now usable interactively during foreground operation.
</action>

<acceptance_criteria>
- `src/main.rs` contains `run_foreground_loop`
- `src/main.rs` contains `handle_stdin_command`
- `handle_stdin_command` contains `strip_prefix("restart ")`
- `handle_stdin_command` contains `supervisor::restart_server`
- `handle_stdin_command` contains `output::print_status_table` (status reprinted after restart)
- `handle_stdin_command` matches on `"status"` and `"help"` commands
- `run_foreground_loop` uses `tokio::select!` with both stdin and shutdown signal branches
- `run_foreground_loop` handles `Ok(None)` (stdin closed) by falling back to signal wait
- `src/main.rs` calls `run_foreground_loop` instead of bare `wait_for_shutdown_signal` in `Commands::Start`
- `cargo build` exits 0
- `cargo clippy -- -D warnings` exits 0
</acceptance_criteria>
</task>

---

## Task 2: Remove dead_code allows from restart_server and SupervisorCommand::Restart

<task id="01-04-T2">
<read_first>
- src/supervisor.rs (the `#[allow(dead_code)]` on `restart_server` and `SupervisorCommand::Restart`)
- src/main.rs (verify `restart_server` is now called from `handle_stdin_command`)
</read_first>

<action>
In `src/supervisor.rs`:

1. Remove `#[allow(dead_code)]` from the `restart_server` function (line 437 area)
2. Remove `#[allow(dead_code)]` from `SupervisorCommand::Restart` variant (line 155 area)
3. Remove the Phase 3 deferral comment on `SupervisorCommand::Restart` — it is now actively used in Phase 1

Update the doc comments on `restart_server` to reflect that it is called from the foreground stdin command reader, not just reserved for Phase 3.
</action>

<acceptance_criteria>
- `src/supervisor.rs` does NOT contain `#[allow(dead_code)]` on `restart_server`
- `src/supervisor.rs` does NOT contain `#[allow(dead_code)]` on `SupervisorCommand::Restart`
- `cargo build` exits 0 (no dead_code warnings since the functions are now used)
- `cargo clippy -- -D warnings` exits 0
</acceptance_criteria>
</task>

---

## Task 3: Add integration test for stdin restart

<task id="01-04-T3">
<read_first>
- tests/cli_integration_test.rs (existing integration tests — follow same patterns)
- src/main.rs (the stdin command interface to test)
- src/supervisor.rs (restart_server behavior)
</read_first>

<action>
Add a new integration test to `tests/cli_integration_test.rs`:

`test_stdin_restart_command`:
- Create a tempfile with TOML config containing two servers:
  ```toml
  [servers.server-a]
  command = "sleep"
  args = ["300"]

  [servers.server-b]
  command = "sleep"
  args = ["300"]
  ```
- Spawn the `mcp-hub` binary as a child process using `std::process::Command` (not `assert_cmd`, since we need to write to stdin):
  - `mcp-hub start --no-color --config <tempfile>`
  - Pipe stdin, stdout, stderr
- Wait briefly (3 seconds) for the status table to be printed (servers reach Running)
- Read initial stdout to confirm both servers appear as "running"
- Write `restart server-a\n` to the child's stdin
- Wait 3 seconds for the restart to complete and a new status table to be printed
- Read stdout — should contain a second status table with "server-a" and "running" (re-spawned)
- Write `status\n` to the child's stdin
- Wait 1 second, read stdout — should contain another status table
- Kill the child process (send SIGTERM or just drop it)
- Assert: stdout contained "server-a" and "server-b" and "running" at least twice (initial + after restart)

Note: This test is inherently timing-sensitive. Use generous timeouts (up to 15s total). If flakiness becomes an issue, the test can be marked `#[ignore]` with a comment explaining it requires manual verification.

Also add `test_stdin_restart_unknown_server`:
- Start the hub with a single server config
- Write `restart nonexistent\n` to stdin
- Read stderr — should contain "not found" error message
- Kill the child

Also add `test_stdin_help_command`:
- Start the hub with a minimal config
- Write `help\n` to stdin
- Read stderr — should contain "Available commands" and "restart" and "status"
- Kill the child
</action>

<acceptance_criteria>
- `tests/cli_integration_test.rs` contains `test_stdin_restart_command`
- `test_stdin_restart_command` writes `restart server-a\n` to stdin of running hub
- `test_stdin_restart_command` asserts stdout contains "server-a" and "running" after restart
- `tests/cli_integration_test.rs` contains `test_stdin_restart_unknown_server`
- `test_stdin_restart_unknown_server` asserts stderr contains "not found"
- `tests/cli_integration_test.rs` contains `test_stdin_help_command`
- `test_stdin_help_command` asserts stderr contains "Available commands"
- `cargo test --test cli_integration_test` exits 0 (all tests pass, including new ones)
</acceptance_criteria>
</task>

---

## Task 4: Final validation

<task id="01-04-T4">
<read_first>
- src/main.rs (full updated implementation)
- src/supervisor.rs (dead_code allows removed)
- tests/cli_integration_test.rs (all tests including new stdin tests)
</read_first>

<action>
Run the complete validation suite:

1. `cargo fmt` — format all code
2. `cargo clippy -- -D warnings` — fix any clippy warnings
3. `cargo test` — run ALL tests (unit + integration, including new stdin tests)
4. Verify no `unwrap()` in any `src/` file: `grep -r "unwrap()" src/` should return nothing

Fix any issues found. Then run the final check:
```bash
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test
```
All three must exit 0.
</action>

<acceptance_criteria>
- `cargo fmt -- --check` exits 0
- `cargo clippy -- -D warnings` exits 0
- `cargo test` exits 0 (all tests pass — config, supervisor, CLI integration including stdin tests)
- `grep -r "unwrap()" src/` returns no results
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
cargo test --test cli_integration_test  # Must exit 0 — integration tests pass (including stdin restart tests)
grep -r "unwrap()" src/           # Must return empty
grep -r "allow(dead_code)" src/supervisor.rs  # Must return empty (dead_code allows removed)
```

Manual smoke test:
```bash
cat > /tmp/test-mcp-hub.toml << 'EOF'
[servers.server-a]
command = "sleep"
args = ["300"]

[servers.server-b]
command = "sleep"
args = ["300"]
EOF

# Start the hub
cargo run -- start --config /tmp/test-mcp-hub.toml

# In the running terminal, type:
#   help           → should print available commands
#   status         → should reprint the status table
#   restart server-a  → should restart server-a, print new status table with fresh PID
#   restart nonexistent → should print "not found" error
#   Ctrl+C         → should shut down all servers and exit 0
```

### must_haves
- [ ] `restart <name>` typed into the running hub's stdin restarts only the named server (PROC-03)
- [ ] After restart, a new status table is printed showing the server running with a fresh PID
- [ ] `restart nonexistent` prints a clear error message to stderr
- [ ] `status` command reprints the current status table
- [ ] `help` command lists available interactive commands
- [ ] Ctrl+C still triggers graceful shutdown (DMN-01 not regressed)
- [ ] When stdin is closed (piped input), the hub falls back to waiting for signals (no crash)
- [ ] `#[allow(dead_code)]` removed from `restart_server` and `SupervisorCommand::Restart`
- [ ] All existing tests still pass (no regressions)
- [ ] New stdin integration tests pass
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] No `unwrap()` in any src/ file
</verification>
