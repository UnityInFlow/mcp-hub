---
plan_id: "01-02"
title: "Process supervisor + backoff + shutdown"
status: complete
executed: 2026-04-02
---

# Plan 02 Execution Summary

## Tasks Completed

### Task 1 — Process spawning with process group isolation
**File:** `src/supervisor.rs`

Created `SpawnedProcess` struct and `spawn_server` function:
- `SpawnedProcess` holds `child: tokio::process::Child`, `pid: u32`, `stdout: Option<ChildStdout>`
- `spawn_server` configures `stdin/stdout/stderr` as `Stdio::piped()`, sets `kill_on_drop(false)`
- On Unix: `cmd.process_group(0)` isolates child from terminal's process group (PROC-08, PITFALL #5)
- Spawns a dedicated stderr drain task via `BufReader::lines()` to prevent pipe-buffer backpressure (PITFALL #2)
- Takes stdout handle and stores it in `SpawnedProcess` for Phase 3 MCP client handoff

Created `shutdown_process`:
- Sends `SIGTERM` to the entire process group via `nix::sys::signal::killpg` on Unix (D-08)
- Races `child.wait()` against `tokio::time::timeout(Duration::from_secs(5))` (D-07)
- On timeout: calls `child.kill().await` then `child.wait().await` to reap zombie (PROC-09)
- All `nix` errors logged at WARN level (ESRCH = benign, process already exited)

### Task 2 — Exponential backoff and supervisor state machine
**File:** `src/supervisor.rs` (continued)

Created `compute_backoff_delay`:
- Caps attempt at `min(10)` before `2^attempt` to prevent u32 overflow (PITFALL #3)
- Formula: `base_delay_secs * 2^capped_attempt`, capped at `max_delay_secs` (60s)
- Jitter via `rand::rng().random_range(0.7..=1.3)` (±30%, thundering herd prevention)
- Floor at 100ms

Created `SupervisorCommand` enum with `Shutdown` and `Restart` variants.

Created `run_server_supervisor`:
- Implements full state machine: `Stopped → Starting → Running → Backoff → Starting …`
- Resolves `env_file` once at startup via `resolve_env`
- Resets `consecutive_failures` to 0 when server ran ≥ `stable_window_secs` (60s) before crashing (D-13)
- Marks `Fatal` after 10 consecutive failures (D-12); exits the loop
- `tokio::select!` on three branches: child exit, cancellation token, command channel
- `Restart` command: stops child via `shutdown_process`, resets `consecutive_failures`, re-enters loop
- Stdout drained in Phase 1 to prevent pipe blocking; Phase 3 replaces drain with MCP client handoff

### Task 3 — Hub orchestrator with Ctrl+C shutdown
**File:** `src/supervisor.rs` (continued), `src/main.rs`

Created `ServerHandle` with `name`, `state_rx` (watch receiver), `cmd_tx` (mpsc sender), `task` (JoinHandle).

Created orchestrator functions:
- `start_all_servers`: spawns one `run_server_supervisor` task per server, returns handles
- `wait_for_initial_states`: polls `state_rx.changed()` per handle until all leave `Stopped`/`Starting`, with configurable timeout (default 10s)
- `stop_all_servers`: sends `Shutdown` to all handles simultaneously (D-09 parallel stop), then awaits all JoinHandles
- `restart_server`: finds handle by name, sends `Restart`; returns `anyhow::bail!` if name not found (PROC-03)

Updated `main.rs`:
- Added `mod supervisor`
- `Commands::Start`: loads config → `start_all_servers` → `wait_for_initial_states` → prints placeholder status table → waits for Ctrl+C/SIGTERM → `shutdown.cancel()` → `stop_all_servers`
- `wait_for_shutdown_signal`: `tokio::select!` on `ctrl_c()` and SIGTERM (via `tokio::signal::unix`) on Unix; Ctrl+C only on Windows

### Task 4 — Supervisor unit tests
**Files:** `tests/supervisor_test.rs`, `tests/fixtures/echo-server.sh`

Created `tests/fixtures/echo-server.sh` (executable) — prints to stderr, sleeps 300s.

Wrote 7 tests all passing:

| Test | What it verifies |
|---|---|
| `test_backoff_delay_increases` | 1s→2s→4s→8s with `jitter_factor=0` |
| `test_backoff_cap_at_60s` | attempt=20 and attempt=100 both ≤ 60s |
| `test_backoff_jitter_in_range` | 100 samples for attempt=2 all in [2.8s, 5.2s], not all identical |
| `test_backoff_attempt_overflow_capped` | `u32::MAX` does not panic, result ≤ 60s |
| `test_spawn_server_with_echo` | Real bash process: pid > 0, kill + wait cleanup |
| `test_spawn_nonexistent_command` | Returns `Err` with "Failed to spawn"/"nonexistent" |
| `test_shutdown_process_terminates_child` | SIGTERM kills `sleep 300`, PID confirmed gone via `nix::signal::kill` |

## Commits

| Commit | Tasks |
|---|---|
| `57156e3 feat(01-02-T1)` | Tasks 1 + 2 + 3 — `supervisor.rs`, `lib.rs`, `main.rs` |
| `38f564b test(01-02-T4)` | Task 4 — `tests/supervisor_test.rs`, `tests/fixtures/echo-server.sh` |

Note: Tasks 1–3 were implemented atomically in a single file (`supervisor.rs`) and committed together. The commit message reflects T1; T2 and T3 content is present in the same commit.

## Verification Results

```
cargo build                       PASS
cargo test --test supervisor_test PASS  (7/7)
cargo test --test config_test     PASS  (7/7)
cargo clippy -- -D warnings       PASS
cargo fmt -- --check              PASS
```

## must_haves Checklist

- [x] `spawn_server` creates child processes with `process_group(0)` on Unix (PROC-08)
- [x] `spawn_server` pipes stdin, stdout, and stderr; stderr is continuously drained (PITFALL #2)
- [x] `shutdown_process` sends SIGTERM first, waits 5s, then SIGKILL (PROC-07, D-07)
- [x] `shutdown_process` kills the entire process group via `killpg`, not just the direct child (D-08)
- [x] `shutdown_process` always calls `child.wait()` after kill to reap zombies (PROC-09)
- [x] `compute_backoff_delay` produces delays of 1s→2s→4s→…→60s (max), with ±30% jitter (PROC-05, D-11)
- [x] Backoff attempt counter is capped at 10 to prevent integer overflow (PITFALL #3)
- [x] `run_server_supervisor` marks server Fatal after 10 consecutive failures (PROC-06, D-12)
- [x] `run_server_supervisor` resets consecutive_failures after 60s of continuous Running (D-13)
- [x] `run_server_supervisor` responds to `Restart` command: stops child, resets state, re-spawns (PROC-03)
- [x] `start_all_servers` spawns all configured servers (PROC-01)
- [x] `stop_all_servers` sends Shutdown to all servers simultaneously (PROC-02, D-09)
- [x] Ctrl+C triggers graceful shutdown via CancellationToken (DMN-01, D-10)
- [x] All 7 supervisor tests pass
- [x] No `unwrap()` in any `src/` file
