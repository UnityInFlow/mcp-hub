---
plan_id: "01-02"
title: "Process supervisor + backoff + shutdown"
phase: 1
wave: 1
depends_on:
  - "01-01"
files_modified:
  - src/supervisor.rs
  - src/types.rs
  - src/main.rs
  - tests/supervisor_test.rs
  - tests/fixtures/echo-server.sh
requirements_addressed:
  - PROC-01
  - PROC-02
  - PROC-03
  - PROC-05
  - PROC-06
  - PROC-07
  - PROC-08
  - PROC-09
  - DMN-01
autonomous: true
---

# Plan 02: Process Supervisor + Backoff + Shutdown

<objective>
Implement the core process supervisor: spawn child processes in isolated process groups, manage a per-server state machine (Stopped -> Starting -> Running -> Backoff -> Fatal), implement exponential backoff with jitter for crash recovery, graceful SIGTERM/SIGKILL shutdown on Ctrl+C, and the `mcp-hub restart <name>` command that restarts a single server while others keep running. This is the heart of the process manager.
</objective>

---

## Task 1: Implement process spawning with process group isolation

<task id="01-02-T1">
<read_first>
- src/types.rs (ProcessState enum, BackoffConfig struct)
- src/config.rs (ServerConfig struct, resolve_env function)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 3: Tokio Process Management — spawn_server code pattern, pipe ownership, process_group(0))
- .planning/research/PITFALLS.md (Pitfall #1: zombie processes, Pitfall #2: pipe blocking, Pitfall #5: signal handling races)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-08: process_group(0), D-07: SIGTERM then SIGKILL)
</read_first>

<action>
Create `src/supervisor.rs` with the following:

1. `pub struct SpawnedProcess` with fields:
   - `child: tokio::process::Child`
   - `pid: u32`

2. `pub fn spawn_server(name: &str, config: &ServerConfig, env: &HashMap<String, String>) -> anyhow::Result<SpawnedProcess>`:
   - Create `tokio::process::Command::new(&config.command)`
   - Set `.args(&config.args)`
   - Set `.envs(env)`
   - Set `.stdin(Stdio::piped())` — reserved for MCP protocol in Phase 3
   - Set `.stdout(Stdio::piped())` — reserved for MCP protocol in Phase 3
   - Set `.stderr(Stdio::piped())`
   - Set `.kill_on_drop(false)` — explicit cleanup only
   - On Unix: `#[cfg(unix)] cmd.process_group(0);` — isolates from terminal process group (PROC-08)
   - If `config.cwd` is Some, set `.current_dir(cwd)`
   - Spawn and capture pid via `child.id().ok_or_else(|| anyhow::anyhow!("Failed to get PID for '{name}'"))?`
   - Take `child.stderr` and spawn a dedicated tokio task that drains stderr line-by-line using `tokio::io::BufReader::new(stderr).lines()`, forwarding each line to `tracing::debug!(server = %name, "{}", line)` — prevents pipe buffer filling (PITFALL #2)
   - Take `child.stdout` and spawn a dedicated tokio task that drains stdout line-by-line (parked for Phase 3 MCP client; for now, drain to prevent blocking)
   - Return `Ok(SpawnedProcess { child, pid })`
   - Error context: `format!("Failed to spawn '{}': {}", name, config.command)`

3. `pub async fn shutdown_process(mut child: tokio::process::Child, pid: u32) -> anyhow::Result<()>`:
   - On Unix: send SIGTERM to the entire process group using `nix::sys::signal::killpg(Pid::from_raw(pid as i32), Signal::SIGTERM)`. Log errors at warn level (process may have already exited — ESRCH is benign).
   - Race `child.wait()` against `tokio::time::timeout(Duration::from_secs(5), ...)`:
     - If child exits within 5 seconds: return Ok
     - If timeout: call `child.kill().await.ok()`, then `child.wait().await.ok()` to reap zombie (PROC-09)
   - On Windows (`#[cfg(windows)]`): just call `child.kill().await.ok()` then `child.wait().await.ok()`.

No `unwrap()` anywhere. All `nix` calls wrapped in `let _ =` or error-logged.
</action>

<acceptance_criteria>
- File `src/supervisor.rs` exists
- `src/supervisor.rs` contains `pub struct SpawnedProcess`
- `SpawnedProcess` has fields `child` and `pid`
- `src/supervisor.rs` contains `pub fn spawn_server`
- `spawn_server` sets `stdin(Stdio::piped())`, `stdout(Stdio::piped())`, `stderr(Stdio::piped())`
- `spawn_server` contains `process_group(0)` inside a `#[cfg(unix)]` block
- `spawn_server` contains `kill_on_drop(false)`
- `spawn_server` spawns a stderr drain task using `BufReader` and `lines()`
- `src/supervisor.rs` contains `pub async fn shutdown_process`
- `shutdown_process` contains `killpg` inside a `#[cfg(unix)]` block
- `shutdown_process` uses `tokio::time::timeout` with `Duration::from_secs(5)`
- `shutdown_process` calls `child.wait().await` after any kill (reap zombie)
- `src/supervisor.rs` does NOT contain `unwrap()`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 2: Implement exponential backoff with jitter

<task id="01-02-T2">
<read_first>
- src/supervisor.rs (spawn_server, shutdown_process — already defined)
- src/types.rs (BackoffConfig struct with defaults)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 5: Exponential Backoff — compute_backoff_delay code pattern)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-11: 1s->2s->4s->...->60s, D-12: 10 failures = Fatal, D-13: reset after 60s running, D-14: Fatal clears on fresh start)
- .planning/research/PITFALLS.md (Pitfall #3: backoff without ceiling or jitter)
</read_first>

<action>
Add to `src/supervisor.rs`:

1. `pub fn compute_backoff_delay(attempt: u32, config: &BackoffConfig) -> std::time::Duration`:
   - Cap attempt at 10 to prevent integer overflow: `let capped_attempt = attempt.min(10);`
   - Calculate base delay: `config.base_delay_secs * (2u32.pow(capped_attempt) as f64)`
   - Cap at max: `base.min(config.max_delay_secs)`
   - Apply jitter: multiply by `rand::Rng::gen_range(rng, (1.0 - config.jitter_factor)..=(1.0 + config.jitter_factor))` where jitter_factor is 0.3 (resulting in range 0.7..=1.3)
   - Floor at 100ms: `.max(0.1)`
   - Return `Duration::from_secs_f64(final_secs)`

2. `pub struct SupervisorCommand` enum with variants:
   - `Shutdown` — stop this server and exit the supervisor task
   - `Restart` — stop this server, clear backoff state, restart from scratch

3. `pub async fn run_server_supervisor(name: String, config: ServerConfig, shutdown: CancellationToken, cmd_rx: tokio::sync::mpsc::Receiver<SupervisorCommand>, state_tx: tokio::sync::watch::Sender<(ProcessState, Option<u32>)>)`:
   - Resolve env via `resolve_env(&config)` (return on error, log, set state to Fatal)
   - Enter a retry loop:
     - Set state to `Starting`, pid to None
     - Call `spawn_server(&name, &config, &env)`
     - On spawn failure: increment `consecutive_failures`. If >= `backoff_config.max_attempts` (10): set state to `Fatal`, return. Otherwise: compute backoff delay, set state to `Backoff { attempt }`, sleep (interruptible by shutdown token or cmd_rx)
     - On spawn success: set state to `Running`, pid to `Some(pid)`. Record `started_at = Instant::now()`
     - Use `tokio::select!` to wait on three futures:
       a. `child.wait()` — child exited on its own. Check if ran >= `stable_window_secs` (60s, D-13) and reset consecutive_failures if so. Increment consecutive_failures. Check Fatal threshold. Compute backoff, set state, sleep (interruptible).
       b. `shutdown.cancelled()` — hub is shutting down. Set state to `Stopping`, call `shutdown_process`, set state to `Stopped`, return.
       c. `cmd_rx.recv()` — received a SupervisorCommand. On `Restart`: stop child, reset consecutive_failures to 0, continue loop. On `Shutdown`: stop child, set state to `Stopped`, return.
   - Fatal state is session-only (D-14) — lives in the loop's local variable, not persisted.

Import `CancellationToken` from `tokio_util::sync`.
</action>

<acceptance_criteria>
- `src/supervisor.rs` contains `pub fn compute_backoff_delay`
- `compute_backoff_delay` contains `attempt.min(10)` (overflow prevention)
- `compute_backoff_delay` contains `gen_range` (jitter)
- `compute_backoff_delay` returns `Duration::from_secs_f64`
- `src/supervisor.rs` contains `pub enum SupervisorCommand` with `Shutdown` and `Restart` variants
- `src/supervisor.rs` contains `pub async fn run_server_supervisor`
- `run_server_supervisor` parameters include `CancellationToken`, `mpsc::Receiver<SupervisorCommand>`, `watch::Sender`
- `run_server_supervisor` contains `tokio::select!` with at least child.wait and shutdown.cancelled branches
- `run_server_supervisor` checks `consecutive_failures >= ` some max_attempts value before setting Fatal
- `run_server_supervisor` resets consecutive_failures when uptime exceeds stable_window_secs
- `src/supervisor.rs` does NOT contain `unwrap()`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 3: Implement hub orchestrator with Ctrl+C shutdown

<task id="01-02-T3">
<read_first>
- src/supervisor.rs (spawn_server, shutdown_process, run_server_supervisor, SupervisorCommand)
- src/config.rs (HubConfig, ServerConfig, find_and_load_config)
- src/types.rs (ProcessState)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 4: Signal Handling — CancellationToken, JoinSet pattern; Section 9: Hub State Architecture — ServerHandle, watch channels)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-09: parallel stop, D-10: Ctrl+C handler, D-15: print status table after launch)
</read_first>

<action>
Add to `src/supervisor.rs`:

1. `pub struct ServerHandle`:
   - `name: String`
   - `state_rx: tokio::sync::watch::Receiver<(ProcessState, Option<u32>)>`
   - `cmd_tx: tokio::sync::mpsc::Sender<SupervisorCommand>`
   - `task: tokio::task::JoinHandle<()>`

2. `pub async fn start_all_servers(config: &HubConfig, shutdown: CancellationToken) -> Vec<ServerHandle>`:
   - For each `(name, server_config)` in `config.servers`:
     - Create `watch::channel((ProcessState::Stopped, None))` for state
     - Create `mpsc::channel(8)` for commands
     - Clone the shutdown token as a child token
     - Spawn `run_server_supervisor` as a tokio task
     - Collect into a `Vec<ServerHandle>`
   - Return the handles

3. `pub async fn wait_for_initial_states(handles: &mut [ServerHandle], timeout: Duration)`:
   - For each handle, wait until state is no longer `Stopped` or `Starting` (i.e., reached `Running`, `Backoff`, or `Fatal`), OR until the timeout expires (use 10 seconds as the default timeout)
   - Use `tokio::time::timeout` wrapping `state_rx.changed()` in a loop
   - This is used by the start command to know when to print the status table (D-15)

4. `pub async fn stop_all_servers(handles: Vec<ServerHandle>)`:
   - Send `SupervisorCommand::Shutdown` to all handles simultaneously via their `cmd_tx` channels (D-09: parallel stop)
   - Await all `handle.task` JoinHandles to completion
   - Log any panicked tasks at warn level

5. `pub async fn restart_server(handles: &[ServerHandle], name: &str) -> anyhow::Result<()>` (PROC-03):
   - Find the handle matching `name`
   - Send `SupervisorCommand::Restart` via `cmd_tx`
   - If name not found, return `anyhow::bail!("Server '{}' not found in config", name)`

Update `src/main.rs` to add `mod supervisor;` and also handle the SIGTERM signal on Unix alongside Ctrl+C:

```rust
// In main, after spawning servers, wait for either:
// - tokio::signal::ctrl_c() (cross-platform)
// - SIGTERM (Unix only, via tokio::signal::unix::signal(SignalKind::terminate()))
// Then cancel the shutdown token
```

The main function should:
1. Parse CLI
2. Load config via `find_and_load_config`
3. Call `start_all_servers`
4. Call `wait_for_initial_states` (with 10s timeout)
5. Print status table (placeholder — Plan 03 adds the formatted table)
6. Wait for Ctrl+C or SIGTERM
7. Cancel shutdown token
8. Wait for all tasks to complete
9. Exit 0
</action>

<acceptance_criteria>
- `src/supervisor.rs` contains `pub struct ServerHandle` with fields `name`, `state_rx`, `cmd_tx`, `task`
- `src/supervisor.rs` contains `pub async fn start_all_servers`
- `start_all_servers` spawns one `run_server_supervisor` task per server in the config
- `src/supervisor.rs` contains `pub async fn wait_for_initial_states`
- `src/supervisor.rs` contains `pub async fn stop_all_servers`
- `stop_all_servers` sends `Shutdown` command to all handles
- `src/supervisor.rs` contains `pub async fn restart_server`
- `restart_server` returns error if server name not found
- `src/main.rs` contains `mod supervisor;`
- `src/main.rs` contains `tokio::signal::ctrl_c`
- `src/supervisor.rs` does NOT contain `unwrap()`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 4: Write supervisor unit tests

<task id="01-02-T4">
<read_first>
- src/supervisor.rs (compute_backoff_delay, spawn_server, shutdown_process, run_server_supervisor — full implementation)
- src/types.rs (BackoffConfig defaults)
- src/config.rs (ServerConfig struct)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 10: unit tests table for supervisor_test)
- .planning/phases/01-config-process-supervisor/01-VALIDATION.md (task 01-02, 01-03, 01-04)
</read_first>

<action>
Create `tests/fixtures/echo-server.sh`:
```bash
#!/bin/bash
# Simple test server that prints to stderr and stays alive
echo "started" >&2
sleep 300
```

Make it executable: `chmod +x tests/fixtures/echo-server.sh`.

Create `tests/supervisor_test.rs` with the following tests:

1. `test_backoff_delay_increases`:
   - Create a `BackoffConfig::default()`
   - Assert `compute_backoff_delay(0, &cfg)` is roughly 1s (between 0.7s and 1.3s due to jitter)
   - Assert `compute_backoff_delay(3, &cfg)` is roughly 8s (between 5.6s and 10.4s)
   - Assert delay for attempt 0 < delay for attempt 3 (run multiple times to account for jitter, check average)
   - Actually: since jitter is random, test without jitter by creating a BackoffConfig with `jitter_factor: 0.0` and then checking exact values:
     - attempt 0: 1.0s
     - attempt 1: 2.0s
     - attempt 2: 4.0s
     - attempt 3: 8.0s

2. `test_backoff_cap_at_60s`:
   - With `jitter_factor: 0.0` config:
   - Assert `compute_backoff_delay(20, &cfg).as_secs_f64() <= 60.0`
   - Assert `compute_backoff_delay(100, &cfg).as_secs_f64() <= 60.0`

3. `test_backoff_jitter_in_range`:
   - With default config (jitter_factor: 0.3):
   - Compute 100 samples for attempt=2 (base = 4.0s)
   - Assert all samples are between `4.0 * 0.7` = 2.8s and `4.0 * 1.3` = 5.2s
   - Assert not all samples are identical (jitter is actually varying)

4. `test_backoff_attempt_overflow_capped`:
   - With `jitter_factor: 0.0` config:
   - `compute_backoff_delay(u32::MAX, &cfg)` should not panic
   - Result should be <= 60.0s

5. `test_spawn_server_with_echo` (async test with `#[tokio::test]`):
   - Create a ServerConfig with `command = "bash"`, `args = ["-c", "echo hello >&2 && sleep 300"]`
   - Call `spawn_server("test-echo", &config, &HashMap::new())`
   - Assert Ok, assert pid > 0
   - Kill the child: `spawned.child.kill().await`
   - Wait: `spawned.child.wait().await`

6. `test_spawn_nonexistent_command`:
   - Create a ServerConfig with `command = "/nonexistent/binary/path"`
   - Call `spawn_server`
   - Assert Err, assert error message contains "Failed to spawn" or "nonexistent"

7. `test_shutdown_process_terminates_child` (async test):
   - Spawn a `sleep 300` process via `spawn_server`
   - Call `shutdown_process(child, pid)`
   - Assert returns Ok
   - Verify the pid is no longer running (on Unix: `nix::sys::signal::kill(Pid::from_raw(pid as i32), None)` returns Err)
</action>

<acceptance_criteria>
- File `tests/fixtures/echo-server.sh` exists and is executable
- File `tests/supervisor_test.rs` exists
- `tests/supervisor_test.rs` contains `test_backoff_delay_increases`
- `tests/supervisor_test.rs` contains `test_backoff_cap_at_60s`
- `tests/supervisor_test.rs` contains `test_backoff_jitter_in_range`
- `tests/supervisor_test.rs` contains `test_backoff_attempt_overflow_capped`
- `tests/supervisor_test.rs` contains `test_spawn_server_with_echo`
- `tests/supervisor_test.rs` contains `test_spawn_nonexistent_command`
- `tests/supervisor_test.rs` contains `test_shutdown_process_terminates_child`
- `cargo test --test supervisor_test` exits 0
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run these commands after all tasks are complete:

```bash
cargo build                       # Must exit 0
cargo test --test supervisor_test # Must exit 0 — all supervisor tests pass
cargo test --test config_test     # Must exit 0 — config tests from Plan 01 still pass
cargo clippy -- -D warnings       # Must exit 0
cargo fmt -- --check              # Must exit 0
```

### must_haves
- [ ] `spawn_server` creates child processes with `process_group(0)` on Unix (PROC-08)
- [ ] `spawn_server` pipes stdin, stdout, and stderr; stderr is continuously drained (PITFALL #2)
- [ ] `shutdown_process` sends SIGTERM first, waits 5s, then SIGKILL (PROC-07, D-07)
- [ ] `shutdown_process` kills the entire process group via `killpg`, not just the direct child (D-08)
- [ ] `shutdown_process` always calls `child.wait()` after kill to reap zombies (PROC-09)
- [ ] `compute_backoff_delay` produces delays of 1s->2s->4s->...->60s (max), with +-30% jitter (PROC-05, D-11)
- [ ] Backoff attempt counter is capped at 10 to prevent integer overflow (PITFALL #3)
- [ ] `run_server_supervisor` marks server Fatal after 10 consecutive failures (PROC-06, D-12)
- [ ] `run_server_supervisor` resets consecutive_failures after 60s of continuous Running (D-13)
- [ ] `run_server_supervisor` responds to `Restart` command: stops child, resets state, re-spawns (PROC-03)
- [ ] `start_all_servers` spawns all configured servers (PROC-01)
- [ ] `stop_all_servers` sends Shutdown to all servers simultaneously (PROC-02, D-09)
- [ ] Ctrl+C triggers graceful shutdown via CancellationToken (DMN-01, D-10)
- [ ] All 7 supervisor tests pass
- [ ] No `unwrap()` in any `src/` file
</verification>
