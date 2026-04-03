---
plan_id: "02-02"
title: "MCP Client + Health Monitor"
phase: 2
wave: 2
depends_on:
  - "02-01"
files_modified:
  - src/supervisor.rs
  - src/mcp/mod.rs
  - src/mcp/health.rs
  - src/types.rs
  - src/config.rs
  - src/main.rs
  - tests/health_monitor.rs
requirements_addressed:
  - PROC-04
  - HLTH-01
  - HLTH-02
  - HLTH-04
  - HLTH-05
autonomous: true
---

# Plan 02-02: MCP Client + Health Monitor

<objective>
Implement the per-server health check loop that sends MCP JSON-RPC pings over stdin/stdout,
tracks health state transitions (Unknown -> Healthy -> Degraded -> Failed), and integrates
with the supervisor's process lifecycle. Extend SpawnedProcess with stdin handle, replace the
stdout drain task with the health check task, wire stderr to the LogAggregator, and upgrade
the watch channel from `(ProcessState, Option<u32>)` to `ServerSnapshot`.
</objective>

---

## Task 1: Extend SpawnedProcess with stdin handle and store it

<task id="02-02-01">
<read_first>
- src/supervisor.rs (SpawnedProcess struct, spawn_server function, lines 1-81)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 4 — stdout ownership handoff)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-12 — dedicated MCP client task per server)
</read_first>

<action>
In `src/supervisor.rs`:

1. Add `use tokio::process::ChildStdin;` to imports.

2. Add `stdin` field to `SpawnedProcess`:
   ```rust
   pub struct SpawnedProcess {
       pub child: tokio::process::Child,
       pub pid: u32,
       pub stdout: Option<ChildStdout>,
       pub stdin: Option<ChildStdin>,  // NEW: for MCP ping writes
   }
   ```

3. In `spawn_server`, capture stdin alongside stdout:
   ```rust
   let stdout = child.stdout.take();
   let stdin = child.stdin.take();   // NEW
   // ...
   Ok(SpawnedProcess { child, pid, stdout, stdin })
   ```

4. Do NOT change the stderr drain task in spawn_server yet (Task 2 handles that).
</action>

<acceptance_criteria>
- grep: `pub stdin: Option<ChildStdin>` in src/supervisor.rs
- grep: `let stdin = child.stdin.take()` in src/supervisor.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Wire stderr to LogAggregator instead of tracing::debug drain

<task id="02-02-02">
<read_first>
- src/supervisor.rs (spawn_server stderr drain at lines 70-78, start_all_servers at lines 361-387)
- src/logs.rs (LogAggregator::push — from Plan 02-01 Task 3)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-08 — logs from stderr only)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 5 — stderr interception)
</read_first>

<action>
1. **Change `spawn_server` signature** to accept an `Option<Arc<LogAggregator>>`:
   ```rust
   pub fn spawn_server(
       name: &str,
       config: &ServerConfig,
       env: &HashMap<String, String>,
       log_agg: Option<std::sync::Arc<crate::logs::LogAggregator>>,
   ) -> anyhow::Result<SpawnedProcess>
   ```

2. **Replace the stderr drain task** in `spawn_server`:
   - If `log_agg` is `Some(agg)`, spawn a task that reads stderr line-by-line and calls `agg.push(name, line).await` for each line, then also prints the formatted line to stderr using `crate::logs::format_log_line`.
   - If `log_agg` is `None`, keep the existing `tracing::debug!` drain (backward compatibility for tests).
   - The task should use a CancellationToken-aware loop is NOT needed here — the stderr pipe closes naturally when the child exits, ending the drain loop.

3. **Update `run_server_supervisor`** signature to accept `Arc<LogAggregator>`:
   ```rust
   pub async fn run_server_supervisor(
       name: String,
       config: ServerConfig,
       shutdown: CancellationToken,
       mut cmd_rx: tokio::sync::mpsc::Receiver<SupervisorCommand>,
       state_tx: tokio::sync::watch::Sender<ServerSnapshot>,
       log_agg: std::sync::Arc<crate::logs::LogAggregator>,
   )
   ```
   Pass `Some(Arc::clone(&log_agg))` to `spawn_server` calls within the supervisor loop.

4. **Update `start_all_servers`** to accept and pass `Arc<LogAggregator>`:
   ```rust
   pub async fn start_all_servers(
       config: &HubConfig,
       shutdown: CancellationToken,
       log_agg: std::sync::Arc<crate::logs::LogAggregator>,
   ) -> Vec<ServerHandle>
   ```
</action>

<acceptance_criteria>
- grep: `log_agg: Option<std::sync::Arc<crate::logs::LogAggregator>>` in src/supervisor.rs (spawn_server)
- grep: `agg.push` in src/supervisor.rs
- grep: `log_agg: std::sync::Arc<crate::logs::LogAggregator>` in src/supervisor.rs (start_all_servers)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Upgrade watch channel to ServerSnapshot

<task id="02-02-03">
<read_first>
- src/supervisor.rs (state_tx sends, ServerHandle struct, run_server_supervisor)
- src/output.rs (collect_states_from_handles, print_status_table — consumers of state_rx)
- src/main.rs (handle_stdin_command — consumer of state_rx)
- src/types.rs (ServerSnapshot from Plan 02-01)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 7 — updated watch channel payload)
</read_first>

<action>
1. **In `src/supervisor.rs`**:
   - Change `state_tx` type from `watch::Sender<(ProcessState, Option<u32>)>` to `watch::Sender<ServerSnapshot>`.
   - Change `ServerHandle.state_rx` to `watch::Receiver<ServerSnapshot>`.
   - In `start_all_servers`, initialize the watch channel with `ServerSnapshot::new(transport)` where `transport` comes from `server_config.transport.clone()`.
   - In `run_server_supervisor`, replace all `state_tx.send((ProcessState::X, pid))` calls with a helper pattern:
     ```rust
     fn update_snapshot(
         tx: &watch::Sender<ServerSnapshot>,
         f: impl FnOnce(&mut ServerSnapshot),
     ) {
         tx.send_modify(f);
     }
     ```
     Use `send_modify` (available since tokio 1.28) to mutate the snapshot in-place:
     - `ProcessState::Starting` -> set process_state, clear pid, clear uptime_since
     - `ProcessState::Running` -> set process_state, set pid, set uptime_since to `Some(Instant::now())`, health to `HealthStatus::Unknown` (D-04)
     - `ProcessState::Backoff` -> set process_state, clear pid, clear uptime_since
     - `ProcessState::Fatal` -> set process_state, clear pid, clear uptime_since
     - `ProcessState::Stopping` -> set process_state (keep pid for display)
     - `ProcessState::Stopped` -> set process_state, clear pid, clear uptime_since
   - Add `restart_count` tracking: add a `total_restarts: u32` local variable, increment on each loop iteration (except the first), update snapshot on each send.

2. **In `src/output.rs`**:
   - Change `collect_states_from_handles` to return `Vec<(String, ServerSnapshot)>`:
     ```rust
     pub fn collect_states_from_handles(
         handles: &[ServerHandle],
     ) -> Vec<(String, ServerSnapshot)>
     ```
     Body: `handle.state_rx.borrow().clone()` (ServerSnapshot implements Clone).
   - Update `print_status_table` signature (handled in Plan 02-03, but the return type change here enables it).

3. **In `src/main.rs`**:
   - Update `handle_stdin_command` to work with the new `collect_states_from_handles` return type. The `print_status_table` call may temporarily use a compatibility shim until Plan 02-03 enhances the table.
   - Temporary shim: extract `(name, snapshot.process_state, snapshot.pid)` tuples for the existing `print_status_table` signature. This will be replaced in Plan 02-03.
</action>

<acceptance_criteria>
- grep: `watch::Sender<ServerSnapshot>` in src/supervisor.rs
- grep: `watch::Receiver<ServerSnapshot>` in src/supervisor.rs
- grep: `send_modify` in src/supervisor.rs
- grep: `total_restarts` in src/supervisor.rs
- grep: `Vec<(String, ServerSnapshot)>` in src/output.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Implement the health check loop (MCP ping task)

<task id="02-02-04">
<read_first>
- src/mcp/protocol.rs (PingRequest, JsonRpcResponse — from Plan 02-01)
- src/types.rs (HealthStatus, compute_health_status, ServerSnapshot — from Plan 02-01)
- src/supervisor.rs (SpawnedProcess with stdin/stdout — from Tasks 1-3)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Sections 1, 3 — health check loop, ping_server)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-12 through D-15 — MCP ping decisions)
- .planning/research/PITFALLS.md (Pitfall #2 — pipe blocking, Pitfall #7 — ID correlation)
</read_first>

<action>
Create `src/mcp/health.rs`:

1. **`ping_server` async function**:
   ```rust
   pub async fn ping_server(
       stdin: &mut ChildStdin,
       stdout_reader: &mut tokio::io::Lines<BufReader<ChildStdout>>,
       id: u64,
   ) -> anyhow::Result<u64>
   ```
   - Serialize `PingRequest::new(id)` to JSON + `"\n"`, write to stdin, flush.
   - `tokio::time::timeout(Duration::from_secs(5), ...)` the stdout read (HLTH-04).
   - Read lines from stdout until we get one that parses as `JsonRpcResponse` with matching `id`.
   - Non-matching lines (unsolicited server output) are logged via `tracing::debug!` and skipped (PITFALL #2 — drain non-ping output to prevent blocking).
   - Return latency in ms on success.
   - Return `anyhow::Error` on timeout, IO error, ID mismatch after reasonable attempts, or error response.

2. **`run_health_check_loop` async function**:
   ```rust
   pub async fn run_health_check_loop(
       server_name: String,
       interval_secs: u64,
       stdin: ChildStdin,
       stdout: ChildStdout,
       snapshot_tx: tokio::sync::watch::Sender<ServerSnapshot>,
       cancel: CancellationToken,
   )
   ```
   - Create `tokio::time::interval(Duration::from_secs(interval_secs))` with `MissedTickBehavior::Skip`.
   - Maintain `request_id: u64` counter starting at 1, incrementing each tick.
   - Maintain `consecutive_misses: u32` counter.
   - Create `BufReader::new(stdout).lines()` for the stdout reader.
   - On each tick:
     - Call `ping_server`.
     - On success: reset `consecutive_misses` to 0, update snapshot health to `HealthStatus::Healthy { latency_ms, last_checked: Instant::now() }` via `snapshot_tx.send_modify(...)`.
     - On failure: increment `consecutive_misses`, compute new status via `compute_health_status(consecutive_misses, &current_health)`, update snapshot.
   - On `cancel.cancelled()`: break out of loop.
   - Log health transitions at `tracing::info!` level.

3. **Update `src/mcp/mod.rs`**:
   ```rust
   pub mod health;
   pub mod protocol;
   ```

4. **Default health check interval constant**:
   ```rust
   pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS: u64 = 30;
   ```
   Defined in `src/mcp/health.rs`.
</action>

<acceptance_criteria>
- grep: `pub async fn ping_server` in src/mcp/health.rs
- grep: `pub async fn run_health_check_loop` in src/mcp/health.rs
- grep: `MissedTickBehavior::Skip` in src/mcp/health.rs
- grep: `timeout.*Duration::from_secs(5)` in src/mcp/health.rs
- grep: `pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS` in src/mcp/health.rs
- grep: `compute_health_status` in src/mcp/health.rs
- grep: `pub mod health` in src/mcp/mod.rs
- No `unwrap()` in src/mcp/health.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 5: Integrate health check into supervisor lifecycle

<task id="02-02-05">
<read_first>
- src/supervisor.rs (run_server_supervisor — current spawn + drain pattern)
- src/mcp/health.rs (run_health_check_loop — from Task 4)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 4 — stdout handoff, Option A for health task lifecycle)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-04 — health resets on restart, D-12 — dedicated task per server, D-13 — configurable interval)
</read_first>

<action>
In `src/supervisor.rs`, modify `run_server_supervisor`:

1. **Replace the stdout drain task** (lines 247-255) with health check task spawning:
   ```rust
   // Spawn the MCP health check task if we have both stdin and stdout.
   // The health task owns stdin (writes pings) and stdout (reads responses).
   // It updates the health field in the ServerSnapshot via send_modify on state_tx.
   let health_cancel = shutdown.child_token();  // per-spawn child token
   if let (Some(stdin), Some(stdout)) = (spawned.stdin, spawned.stdout) {
       let health_name = name.clone();
       let health_tx = state_tx.clone();
       let interval = config.health_check_interval
           .unwrap_or(crate::mcp::health::DEFAULT_HEALTH_CHECK_INTERVAL_SECS);
       let cancel = health_cancel.clone();
       tokio::spawn(async move {
           crate::mcp::health::run_health_check_loop(
               health_name,
               interval,
               stdin,
               stdout,
               health_tx,
               cancel,
           ).await;
       });
   }
   ```

2. **Cancel the health task on process exit or restart**:
   - Before `shutdown_process(child, pid)` calls (in Restart, Shutdown, and crash branches), call `health_cancel.cancel()` to stop the health check loop for the dying process.
   - This ensures the old health task releases its stdin/stdout handles before the new process is spawned.

3. **Reset health on re-spawn** (D-04):
   - When transitioning to `ProcessState::Starting` at the top of the loop, ensure `health` is set to `HealthStatus::Unknown` in the snapshot. This is already handled by the `send_modify` for Starting state (from Task 3), but verify it explicitly.

4. **The `health_cancel` token pattern**:
   - Create `health_cancel` as a child of the main `shutdown` token at the start of each loop iteration.
   - When the main shutdown fires, child tokens are automatically cancelled.
   - When a Restart command arrives, explicitly cancel `health_cancel` before calling `shutdown_process`.

5. **Handle the case where stdin/stdout are None** (should not happen in normal operation):
   - If `spawned.stdin` or `spawned.stdout` is None, log a warning and skip health checks for that server. The server will stay at `HealthStatus::Unknown`. Still drain stdout if only stdout is available (fallback drain to prevent blocking).
</action>

<acceptance_criteria>
- grep: `run_health_check_loop` in src/supervisor.rs
- grep: `health_cancel` in src/supervisor.rs
- grep: `health_cancel.cancel()` in src/supervisor.rs
- grep: `HealthStatus::Unknown` in src/supervisor.rs
- grep: `DEFAULT_HEALTH_CHECK_INTERVAL_SECS` in src/supervisor.rs
- No stdout drain task with tracing::debug![stdout] remaining (the old drain is replaced)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 6: Update main.rs to create and pass LogAggregator

<task id="02-02-06">
<read_first>
- src/main.rs (Commands::Start branch, run_foreground_loop)
- src/logs.rs (LogAggregator::new — from Plan 02-01)
- src/supervisor.rs (start_all_servers — updated signature from Task 2)
</read_first>

<action>
In `src/main.rs`, update the `Commands::Start` branch:

1. **Create the LogAggregator** before `start_all_servers`:
   ```rust
   use std::sync::Arc;
   use crate::logs::LogAggregator;

   let server_names: Vec<String> = config.servers.keys().cloned().collect();
   let log_agg = Arc::new(LogAggregator::new(&server_names, 10_000));
   ```

2. **Pass `Arc::clone(&log_agg)` to `start_all_servers`**:
   ```rust
   let mut handles = supervisor::start_all_servers(
       &config, shutdown.clone(), Arc::clone(&log_agg)
   ).await;
   ```

3. **Pass `Arc<LogAggregator>` into `run_foreground_loop`** (for the `logs` stdin command in Plan 02-03):
   ```rust
   run_foreground_loop(&handles, color, Arc::clone(&log_agg)).await?;
   ```
   Update `run_foreground_loop` signature to accept the third parameter (stored but not fully used until Plan 02-03).

4. **Ensure `mod logs;` and `mod mcp;` are declared** at the top of main.rs (should already be done in Plan 02-01, verify).
</action>

<acceptance_criteria>
- grep: `LogAggregator::new` in src/main.rs
- grep: `Arc::clone(&log_agg)` in src/main.rs
- grep: `log_agg` in src/main.rs (run_foreground_loop call)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 7: Unit and integration tests for health monitoring

<task id="02-02-07">
<read_first>
- src/mcp/health.rs (ping_server, run_health_check_loop — from Task 4)
- src/mcp/protocol.rs (PingRequest, JsonRpcResponse)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 13 — test strategy)
</read_first>

<action>
Create `tests/health_monitor.rs`:

1. **PingRequest serialization test**:
   - Serialize `PingRequest::new(42)` and verify JSON contains `"jsonrpc":"2.0"`, `"method":"ping"`, `"id":42`.

2. **JsonRpcResponse deserialization tests**:
   - Successful response: `{"jsonrpc":"2.0","result":{},"id":1}` -> id=1, result=Some, error=None
   - Error response: `{"jsonrpc":"2.0","error":{"code":-32601},"id":1}` -> error=Some
   - Missing fields handled gracefully (serde allows missing optional fields)

3. **Health check integration test with mock MCP server**:
   Create a test helper that spawns a child process (using a small bash script or inline Rust test binary) that responds to JSON-RPC pings on stdin/stdout:
   - Test fixture: `tests/fixtures/ping-responder.sh` — reads stdin line-by-line, parses JSON to extract `id`, echoes back `{"jsonrpc":"2.0","result":{},"id":<id>}`
   - Alternative: use `tokio::process::Command::new("bash").arg("-c").arg(script)` inline.
   - Test: spawn the responder, call `ping_server` with its stdin/stdout, verify Ok(latency_ms) returned.

4. **Timeout test**:
   - Spawn a child that reads stdin but never responds (e.g., `cat > /dev/null`).
   - Call `ping_server` and verify it returns an error within ~5 seconds.
   - Use `#[tokio::test]` with a reasonable timeout attribute.

5. **Health state transition integration**:
   - Test `run_health_check_loop` with a very short interval (100ms for testing) against a mock responder.
   - Verify that the watch channel receives `HealthStatus::Healthy` after the first successful ping.
   - Cancel the token and verify the loop exits cleanly.

Note: Some tests may need `#[cfg(unix)]` if they use bash. Provide `#[cfg(not(unix))]` stubs or skip annotations for Windows.
</action>

<acceptance_criteria>
- grep: `fn ping_request_serialization` in tests/health_monitor.rs
- grep: `fn successful_ping_response` in tests/health_monitor.rs
- grep: `fn ping_timeout` in tests/health_monitor.rs
- cargo test --test health_monitor passes on the current platform
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
- [ ] SpawnedProcess has `stdin: Option<ChildStdin>` field
- [ ] stderr lines flow to LogAggregator (not just tracing::debug)
- [ ] watch channel carries ServerSnapshot (not tuple)
- [ ] ServerSnapshot includes process_state, health, pid, uptime_since, restart_count, transport
- [ ] restart_count increments on each re-spawn and never resets
- [ ] Health check loop sends JSON-RPC ping on stdin, reads response from stdout
- [ ] 5-second timeout per ping (HLTH-04) via tokio::time::timeout
- [ ] MissedTickBehavior::Skip prevents burst pings after sleep/suspend
- [ ] Health task uses per-spawn CancellationToken, cancelled on process exit or restart
- [ ] Health resets to Unknown on server restart (D-04)
- [ ] Non-ping stdout lines are drained (not blocking pipe — PITFALL #2)
- [ ] Configurable health check interval via config.health_check_interval or default 30s
- [ ] LogAggregator created in main.rs and passed through to supervisor
- [ ] All existing Phase 1 tests still pass
- [ ] All new health_monitor tests pass
- [ ] cargo clippy -D warnings passes
- [ ] No unwrap() in production code (src/)
</verification>
