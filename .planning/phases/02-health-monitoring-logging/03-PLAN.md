---
plan_id: "02-03"
title: "Status Command + Logs Command + Integration"
phase: 2
wave: 3
depends_on:
  - "02-01"
  - "02-02"
files_modified:
  - src/output.rs
  - src/cli.rs
  - src/main.rs
  - tests/status_table.rs
  - tests/logs_command.rs
  - tests/integration_phase2.rs
requirements_addressed:
  - PROC-04
  - LOG-01
  - LOG-02
autonomous: true
---

# Plan 02-03: Status Command + Logs Command + Integration

<objective>
Enhance the status table to display all Phase 2 columns (Name, Process State, Health, PID,
Uptime, Restarts, Transport), add the `mcp-hub logs` CLI subcommand and the `logs`/`status`
stdin commands in foreground mode, and write integration tests that verify the full Phase 2
feature set end-to-end.
</objective>

---

## Task 1: Enhance the status table with Phase 2 columns

<task id="02-03-01">
<read_first>
- src/output.rs (current print_status_table, collect_states_from_handles)
- src/types.rs (ServerSnapshot, HealthStatus, ProcessState, format_uptime — from Plans 02-01/02-02)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-09 — table columns, D-10 — uptime format)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 7 — status table enhancement)
</read_first>

<action>
In `src/output.rs`:

1. **Replace `print_status_table` signature and implementation**:
   ```rust
   pub fn print_status_table(servers: &[(String, ServerSnapshot)], color: bool)
   ```

2. **New table header** per D-09:
   ```rust
   table.set_header(vec!["Name", "State", "Health", "PID", "Uptime", "Restarts", "Transport"]);
   ```

3. **For each row**, extract from `ServerSnapshot`:
   - **Name**: server name string
   - **State**: `snapshot.process_state.to_string()` with color (existing color logic)
   - **Health**: `snapshot.health.to_string()` with health-specific colors:
     - `Healthy` -> Green
     - `Degraded` -> Yellow
     - `Failed` -> Red
     - `Unknown` -> DarkGrey
   - **PID**: `snapshot.pid.map(|p| p.to_string()).unwrap_or("-".into())`
   - **Uptime**: if `snapshot.uptime_since` is Some, compute `format_uptime(since.elapsed())`, else `"-"`
   - **Restarts**: `snapshot.restart_count.to_string()`
   - **Transport**: `&snapshot.transport`

4. **Update `collect_states_from_handles`** to return `Vec<(String, ServerSnapshot)>`:
   ```rust
   pub fn collect_states_from_handles(
       handles: &[ServerHandle],
   ) -> Vec<(String, ServerSnapshot)> {
       handles.iter().map(|h| {
           let snapshot = h.state_rx.borrow().clone();
           (h.name.clone(), snapshot)
       }).collect()
   }
   ```

5. **Remove any temporary shim** from Plan 02-02 Task 3 that adapted the old signature.
</action>

<acceptance_criteria>
- grep: `"Health"` in src/output.rs (table header)
- grep: `"Uptime"` in src/output.rs (table header)
- grep: `"Restarts"` in src/output.rs (table header)
- grep: `"Transport"` in src/output.rs (table header)
- grep: `format_uptime` in src/output.rs
- grep: `fn print_status_table.*ServerSnapshot` in src/output.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Add `Logs` and `Status` CLI subcommands to cli.rs

<task id="02-03-02">
<read_first>
- src/cli.rs (current Commands enum, RestartArgs)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 8 — mcp-hub logs CLI subcommand)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-06 — logs behavior, D-11 — status behavior)
</read_first>

<action>
In `src/cli.rs`:

1. **Add `LogsArgs` struct**:
   ```rust
   #[derive(Debug, clap::Args)]
   pub struct LogsArgs {
       /// Follow log output (streams new lines). Requires daemon mode (Phase 3).
       #[arg(short = 'f', long)]
       pub follow: bool,

       /// Filter to a specific server by name.
       #[arg(long, short = 's', value_name = "NAME")]
       pub server: Option<String>,

       /// Number of recent lines to show (default: 100).
       #[arg(long, short = 'n', default_value = "100")]
       pub lines: usize,
   }
   ```

2. **Add variants to `Commands` enum**:
   ```rust
   pub enum Commands {
       Start,
       Stop,
       Restart(RestartArgs),

       /// Show status of all servers (name, state, health, PID, uptime, restarts).
       Status,

       /// Show server logs.
       Logs(LogsArgs),
   }
   ```
</action>

<acceptance_criteria>
- grep: `pub struct LogsArgs` in src/cli.rs
- grep: `Status,` in src/cli.rs (in Commands enum)
- grep: `Logs(LogsArgs)` in src/cli.rs
- grep: `pub follow: bool` in src/cli.rs
- grep: `pub server: Option<String>` in src/cli.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Handle Status and Logs CLI subcommands in main.rs

<task id="02-03-03">
<read_first>
- src/main.rs (match cli.command, handle_stdin_command)
- src/cli.rs (Commands::Status, Commands::Logs — from Task 2)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-06, D-11 — Phase 2 behavior for separate process invocation)
</read_first>

<action>
In `src/main.rs`:

1. **Add `Commands::Status` handler** in the main match:
   ```rust
   Commands::Status => {
       // Phase 2: requires daemon IPC (Phase 3).
       // In foreground mode, status is available via typing 'status' in the terminal.
       eprintln!(
           "mcp-hub status: no daemon running.\n\
            In foreground mode, type 'status' in the running hub's terminal.\n\
            Daemon-mode status will be available in a future version."
       );
       std::process::exit(1);
   }
   ```

2. **Add `Commands::Logs` handler** in the main match:
   ```rust
   Commands::Logs(args) => {
       // Phase 2: requires daemon IPC (Phase 3) for separate process access.
       // In foreground mode, logs are visible in the terminal output directly.
       if args.follow {
           eprintln!(
               "mcp-hub logs --follow: requires daemon mode.\n\
                In foreground mode, server logs stream to the terminal automatically.\n\
                Daemon-mode log streaming will be available in a future version."
           );
       } else {
           eprintln!(
               "mcp-hub logs: no daemon running.\n\
                In foreground mode, type 'logs' in the running hub's terminal to dump recent logs.\n\
                Daemon-mode log access will be available in a future version."
           );
       }
       std::process::exit(1);
   }
   ```
</action>

<acceptance_criteria>
- grep: `Commands::Status` in src/main.rs
- grep: `Commands::Logs` in src/main.rs
- grep: `no daemon running` in src/main.rs (both handlers)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Add `logs` and enhanced `status` stdin commands to foreground loop

<task id="02-03-04">
<read_first>
- src/main.rs (handle_stdin_command, run_foreground_loop)
- src/logs.rs (LogAggregator — snapshot_all, get_buffer, format_log_line)
- src/output.rs (print_status_table, collect_states_from_handles — updated in Task 1)
</read_first>

<action>
In `src/main.rs`:

1. **Update `run_foreground_loop` signature** to accept `Arc<LogAggregator>`:
   ```rust
   async fn run_foreground_loop(
       handles: &[supervisor::ServerHandle],
       color: bool,
       log_agg: std::sync::Arc<crate::logs::LogAggregator>,
   ) -> anyhow::Result<()>
   ```

2. **Update `handle_stdin_command` signature** to accept `Arc<LogAggregator>`:
   ```rust
   async fn handle_stdin_command(
       input: &str,
       handles: &[supervisor::ServerHandle],
       color: bool,
       log_agg: &crate::logs::LogAggregator,
   )
   ```

3. **Add `logs` command handling** in `handle_stdin_command`:
   ```rust
   } else if input == "logs" || input.starts_with("logs ") {
       // "logs" -> dump last 100 lines from all servers
       // "logs <name>" -> dump last 100 lines for specific server
       let parts: Vec<&str> = input.splitn(2, ' ').collect();
       if parts.len() == 1 {
           // All servers
           let lines = log_agg.snapshot_all().await;
           let tail = if lines.len() > 100 { &lines[lines.len()-100..] } else { &lines };
           for line in tail {
               println!("{}", crate::logs::format_log_line(line, color));
           }
           if lines.is_empty() {
               eprintln!("No logs captured yet.");
           }
       } else {
           let server_name = parts[1].trim();
           match log_agg.get_buffer(server_name) {
               Some(buf) => {
                   let lines = buf.snapshot_last(100).await;
                   for line in &lines {
                       println!("{}", crate::logs::format_log_line(line, color));
                   }
                   if lines.is_empty() {
                       eprintln!("No logs captured for '{server_name}' yet.");
                   }
               }
               None => {
                   eprintln!("Unknown server: '{server_name}'");
               }
           }
       }
   }
   ```

4. **Update the existing `status` command** to use the new `print_status_table`:
   - Already calls `collect_states_from_handles` and `print_status_table` — ensure these use the new signatures from Task 1.

5. **Update the `help` command** to include `logs`:
   ```
   eprintln!("  logs            - Show recent logs from all servers");
   eprintln!("  logs <name>     - Show recent logs for a specific server");
   ```

6. **Update the hint line** at the start of foreground loop:
   ```
   eprintln!("Type 'help' for available commands, or press Ctrl+C to stop.");
   ```
   (This is already present, just ensure logs is documented in help.)

7. **Pass `log_agg` through** all the call sites: `run_foreground_loop` body passes `&*log_agg` to `handle_stdin_command`.
</action>

<acceptance_criteria>
- grep: `"logs"` in src/main.rs (handle_stdin_command)
- grep: `snapshot_all` in src/main.rs
- grep: `snapshot_last` in src/main.rs
- grep: `format_log_line` in src/main.rs
- grep: `logs <name>` in src/main.rs (help text)
- grep: `log_agg` in src/main.rs (run_foreground_loop signature)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 5: Unit tests for the enhanced status table

<task id="02-03-05">
<read_first>
- src/output.rs (print_status_table, collect_states_from_handles — from Task 1)
- src/types.rs (ServerSnapshot, HealthStatus, ProcessState, format_uptime)
</read_first>

<action>
Create `tests/status_table.rs`:

1. **`status_table_has_all_columns`**:
   - Construct a `Vec<(String, ServerSnapshot)>` with 2 servers: one Running+Healthy, one Backoff+Degraded.
   - Capture stdout from `print_status_table` (redirect or check that it does not panic).
   - Since `print_status_table` prints to stdout, use a simpler assertion: verify it does not panic with valid data.

2. **`status_table_running_healthy_server`**:
   - Construct a snapshot: Running, Healthy with latency 42ms, PID 1234, uptime 3661s, 0 restarts, "stdio" transport.
   - Call `print_status_table` with color=false. Capture output if possible, or just verify no panic.

3. **`status_table_fatal_failed_server`**:
   - Construct a snapshot: Fatal, Failed with 7 misses, no PID, no uptime, 5 restarts, "stdio" transport.
   - Verify no panic.

4. **`status_table_empty_servers`**:
   - Call with empty slice. Verify no panic (prints header only).

5. **`collect_states_snapshot_format`**:
   - Create mock `ServerHandle`s with watch channels seeded with known ServerSnapshots.
   - Call `collect_states_from_handles` and verify the returned tuples match expectations.

Note: For tests that need to verify table output content, consider using `assert_cmd` or capturing stdout in a buffer. If `print_status_table` is hard to test directly (prints to real stdout), add a `format_status_table` variant that returns a `String` and test that instead. Make `print_status_table` a thin wrapper around `format_status_table`.
</action>

<acceptance_criteria>
- grep: `fn status_table_has_all_columns` in tests/status_table.rs
- grep: `fn status_table_running_healthy_server` in tests/status_table.rs
- grep: `fn status_table_empty_servers` in tests/status_table.rs
- cargo test --test status_table passes
</acceptance_criteria>
</task>

---

## Task 6: Tests for logs CLI subcommand and stdin logs command

<task id="02-03-06">
<read_first>
- src/main.rs (Commands::Logs handler, handle_stdin_command logs branch — from Tasks 3-4)
- src/cli.rs (LogsArgs — from Task 2)
- src/logs.rs (LogAggregator, format_log_line)
</read_first>

<action>
Create `tests/logs_command.rs`:

1. **`logs_subcommand_prints_daemon_required`** (assert_cmd):
   - Run `mcp-hub logs` as a subprocess.
   - Verify exit code is 1.
   - Verify stderr contains "no daemon running".

2. **`logs_follow_subcommand_prints_daemon_required`** (assert_cmd):
   - Run `mcp-hub logs --follow` as a subprocess.
   - Verify exit code is 1.
   - Verify stderr contains "requires daemon mode".

3. **`logs_subcommand_with_server_filter`** (assert_cmd):
   - Run `mcp-hub logs --server foo` as a subprocess.
   - Verify exit code is 1 (no daemon).

4. **`status_subcommand_prints_daemon_required`** (assert_cmd):
   - Run `mcp-hub status` as a subprocess.
   - Verify exit code is 1.
   - Verify stderr contains "no daemon running".

5. **`logs_args_parsing`** (unit test):
   - Parse `Cli` from `["mcp-hub", "logs", "-f", "-s", "my-server", "-n", "50"]`.
   - Verify `follow=true`, `server=Some("my-server")`, `lines=50`.

6. **`logs_args_defaults`** (unit test):
   - Parse `Cli` from `["mcp-hub", "logs"]`.
   - Verify `follow=false`, `server=None`, `lines=100`.
</action>

<acceptance_criteria>
- grep: `fn logs_subcommand_prints_daemon_required` in tests/logs_command.rs
- grep: `fn status_subcommand_prints_daemon_required` in tests/logs_command.rs
- grep: `fn logs_args_parsing` in tests/logs_command.rs
- cargo test --test logs_command passes
</acceptance_criteria>
</task>

---

## Task 7: Full Phase 2 integration test

<task id="02-03-07">
<read_first>
- src/main.rs (full start flow)
- src/supervisor.rs (start_all_servers, ServerHandle, ServerSnapshot)
- src/mcp/health.rs (run_health_check_loop)
- src/logs.rs (LogAggregator)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 13 — integration test strategy)
- tests/ (existing Phase 1 tests for patterns)
</read_first>

<action>
Create `tests/integration_phase2.rs`:

1. **Create a test fixture**: `tests/fixtures/ping-responder.sh`
   ```bash
   #!/usr/bin/env bash
   # Minimal MCP server that responds to JSON-RPC pings on stdin.
   # stderr output simulates server logs.
   echo "Ping responder started" >&2
   while IFS= read -r line; do
       id=$(echo "$line" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id', 0))" 2>/dev/null)
       if [ -n "$id" ] && [ "$id" != "0" ]; then
           echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
       fi
       echo "Received request id=$id" >&2
   done
   ```
   Make it executable: `chmod +x`.

2. **`test_healthy_server_status_table`** (`#[cfg(unix)]`):
   - Write a temporary `mcp-hub.toml` with one server using `ping-responder.sh`.
   - Use `assert_cmd` to run `mcp-hub start --config <path>`.
   - Send `status\n` to stdin after a brief delay.
   - Verify the output table contains "running" and "healthy" (or "unknown" if health check hasn't run yet).
   - Send Ctrl+C (or close stdin) to stop.
   Note: This is complex to orchestrate with assert_cmd. Alternative approach: use the programmatic API directly in a `#[tokio::test]`.

3. **`test_log_aggregation_from_stderr`** (`#[tokio::test]`, `#[cfg(unix)]`):
   - Create a `HubConfig` with one server: `bash -c 'echo "log line 1" >&2; echo "log line 2" >&2; sleep 60'`.
   - Create `LogAggregator`, call `start_all_servers`.
   - Wait 2 seconds for stderr lines to be captured.
   - Call `log_agg.get_buffer("test-server").snapshot(100).await`.
   - Verify at least 2 log lines captured with correct server name.
   - Shut down cleanly.

4. **`test_health_transitions_to_healthy`** (`#[tokio::test]`, `#[cfg(unix)]`):
   - Create a `HubConfig` with one server using `ping-responder.sh` and `health_check_interval: 1` (1 second for fast test).
   - Create `LogAggregator`, call `start_all_servers`.
   - Wait for ~3 seconds.
   - Read `ServerSnapshot` from the handle's `state_rx`.
   - Verify `health` is `HealthStatus::Healthy`.
   - Shut down cleanly.

5. **`test_health_degrades_on_unresponsive_server`** (`#[tokio::test]`, `#[cfg(unix)]`):
   - Create a `HubConfig` with one server: `bash -c 'cat > /dev/null'` (reads stdin but never responds).
   - Set `health_check_interval: 1`.
   - Wait for ~4 seconds (enough for 2 missed pings at 1s interval).
   - Read `ServerSnapshot` from the handle's `state_rx`.
   - Verify `health` is `HealthStatus::Degraded` (or at least not Healthy).
   - Shut down cleanly.

6. **`test_status_table_output_has_seven_columns`** (`#[cfg(unix)]`):
   - Use `assert_cmd` to start `mcp-hub` with a simple config, pipe `status\n` to stdin.
   - Verify output contains all 7 column headers: Name, State, Health, PID, Uptime, Restarts, Transport.

Important: All integration tests must clean up child processes. Use `CancellationToken` + `stop_all_servers` in a `drop` guard or explicit cleanup block. Consider using `tokio::time::timeout` around the entire test to prevent hangs.
</action>

<acceptance_criteria>
- file exists: tests/fixtures/ping-responder.sh
- grep: `fn test_log_aggregation_from_stderr` in tests/integration_phase2.rs
- grep: `fn test_health_transitions_to_healthy` in tests/integration_phase2.rs
- grep: `fn test_health_degrades_on_unresponsive_server` in tests/integration_phase2.rs
- cargo test --test integration_phase2 passes (at least on Unix)
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run in sequence:

```bash
cargo build 2>&1 | head -5
cargo clippy -- -D warnings 2>&1 | head -10
cargo test 2>&1
cargo fmt -- --check 2>&1 | head -5
```

### must_haves
- [ ] Status table displays 7 columns: Name, State, Health, PID, Uptime, Restarts, Transport (PROC-04 complete)
- [ ] Health column shows colored health status (Healthy=green, Degraded=yellow, Failed=red, Unknown=grey)
- [ ] Uptime displays as HH:MM:SS format (D-10)
- [ ] `mcp-hub logs` CLI subcommand exists and prints "daemon required" message for Phase 2
- [ ] `mcp-hub status` CLI subcommand exists and prints "daemon required" message for Phase 2
- [ ] `logs` stdin command in foreground mode dumps ring buffer with colored prefixes (LOG-01, LOG-02)
- [ ] `logs <name>` stdin command filters to specific server (LOG-02)
- [ ] Each log line has timestamp and server name prefix (LOG-03)
- [ ] Logs come from stderr only — stdout reserved for MCP protocol (LOG-04)
- [ ] Ring buffer stores 10,000 lines per server (LOG-05)
- [ ] LogsArgs parses --follow, --server, --lines flags correctly
- [ ] help command lists logs and status
- [ ] Integration test confirms healthy server shows HealthStatus::Healthy
- [ ] Integration test confirms unresponsive server shows HealthStatus::Degraded
- [ ] Integration test confirms stderr log lines are captured in LogAggregator
- [ ] All Phase 1 tests still pass (no regressions)
- [ ] All Phase 2 tests pass
- [ ] cargo clippy -D warnings passes
- [ ] cargo fmt --check passes
- [ ] No unwrap() in production code (src/)
</verification>
