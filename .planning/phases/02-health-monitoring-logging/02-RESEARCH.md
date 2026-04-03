# Phase 2: Health Monitoring & Logging — Research

**Written:** 2026-04-02
**For:** Planning Phase 2 (Health Monitoring & Logging)
**Requirements in scope:** PROC-04, HLTH-01, HLTH-02, HLTH-03, HLTH-04, HLTH-05, LOG-01, LOG-02, LOG-03, LOG-04, LOG-05

---

## 1. MCP JSON-RPC 2.0 Ping Protocol

### What the MCP spec says about ping

MCP uses JSON-RPC 2.0 over stdio. The `ping` method is an optional but standard health check. Decision D-15 codifies the exact wire format:

```json
{"jsonrpc":"2.0","method":"ping","id":1}
```

Expected response (within 5s per D-13/HLTH-04):

```json
{"jsonrpc":"2.0","result":{},"id":1}
```

Key facts from ARCHITECTURE.md and PITFALLS.md:

- **No MCP SDK needed.** The protocol is simple enough to implement with `serde_json` alone. Phase 2 only needs `ping` — full introspection (`initialize`, `tools/list` etc.) is Phase 3.
- **ID correlation is required even for ping.** Pitfall #7 warns that responses may arrive out of order. For a single ping-at-a-time health check loop (Phase 2 model), a monotonically incrementing `u64` counter per server is sufficient. A full `HashMap<id, oneshot::Sender>` dispatcher is deferred to Phase 3 when concurrent requests are needed.
- **Ping timeout must not block other servers.** Each server runs its health check in an independent tokio task with `tokio::time::timeout(Duration::from_secs(5), ...)`.
- **Non-response = missed ping, not error.** A timeout increments the missed_pings counter. The process crashing is handled by the supervisor independently — HealthStatus is orthogonal to ProcessState (D-01).
- **stdout is the MCP channel.** Phase 1 currently drains stdout silently into `tracing::debug!`. Phase 2 replaces that drain task with the MCP ping client task, which owns the stdout `BufReader`. stderr stays with the log aggregator.

### Minimal Phase 2 JSON-RPC types needed

```rust
// src/mcp/protocol.rs
#[derive(Serialize)]
struct PingRequest {
    jsonrpc: &'static str,   // "2.0"
    method: &'static str,    // "ping"
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
```

Phase 3 will expand these into full `initialize`/`tools/list` request/response types. For Phase 2, only `PingRequest` and a generic `JsonRpcResponse` are needed.

### stdin write path

The child's stdin handle (`ChildStdin`) is separate from stdout. Phase 1 does not use stdin at all. Phase 2 needs to write newline-terminated JSON-RPC to it. `tokio::io::AsyncWriteExt::write_all` + `\n` delimiter:

```rust
let msg = serde_json::to_string(&ping_req)? + "\n";
stdin.write_all(msg.as_bytes()).await?;
```

The `ChildStdin` handle must be stored in `SpawnedProcess` (or returned from `spawn_server`) alongside `stdout`. Currently `SpawnedProcess` only stores `stdout`. Phase 2 adds `stdin`.

---

## 2. HealthStatus State Machine

### Separate from ProcessState — D-01

`ProcessState` answers: "Is the OS process alive?"
`HealthStatus` answers: "Is the MCP server responding?"

A server can be `ProcessState::Running` but `HealthStatus::Degraded` (process is alive but not responding to pings). This separation is Pitfall #4 explicitly.

### State transitions per D-02, D-03, D-04, D-05

```
Unknown -> (first ping succeeds) -> Healthy
Healthy -> (2 consecutive missed pings) -> Degraded
Degraded -> (5 more consecutive missed pings) -> Failed   (total: 7 missed at 30s = ~3.5 min)
Degraded -> (any ping succeeds) -> Healthy
Failed -> (any ping succeeds) -> Healthy
* -> (server restarts) -> Unknown                          (D-04)
```

Degraded does NOT trigger auto-restart. Only ProcessState::Fatal triggers "stop restarting." Health transitions are informational and displayed in the status table.

### Implementation shape

```rust
// src/types.rs addition
#[derive(Debug, Clone, Default)]
pub enum HealthStatus {
    #[default]
    Unknown,
    Healthy { latency_ms: u64, last_checked: std::time::Instant },
    Degraded { consecutive_misses: u32, last_success: Option<std::time::Instant> },
    Failed { consecutive_misses: u32 },
}
```

`HealthStatus` needs a `watch::Sender` per server (mirrors the existing `state_tx: watch::Sender<(ProcessState, Option<u32>)>` in `ServerHandle`). The output layer subscribes to both watch channels to render the status table.

Option: combine them into a single watch channel sending `ServerSnapshot { process_state, pid, health, uptime_since, restart_count }`. This avoids two separate subscriptions and reduces watch channel proliferation. Recommended — the output layer always needs all fields together anyway.

---

## 3. Health Check Loop — Per-Server Tokio Task

### Pattern: tokio::time::interval

```rust
// In health.rs or mcp/health.rs
pub async fn run_health_check_loop(
    server_name: String,
    interval_secs: u64,
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    health_tx: watch::Sender<HealthStatus>,
    shutdown: CancellationToken,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut request_id: u64 = 0;
    let mut consecutive_misses: u32 = 0;
    let mut stdout_reader = BufReader::new(stdout);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                request_id += 1;
                match ping_server(&mut stdin, &mut stdout_reader, request_id).await {
                    Ok(latency_ms) => {
                        consecutive_misses = 0;
                        let _ = health_tx.send(HealthStatus::Healthy { latency_ms, ... });
                    }
                    Err(_) => {
                        consecutive_misses += 1;
                        // Apply D-02 / D-03 thresholds
                        let new_status = compute_health_status(consecutive_misses, &health_tx.borrow());
                        let _ = health_tx.send(new_status);
                    }
                }
            }
            _ = shutdown.cancelled() => break,
        }
    }
}
```

`MissedTickBehavior::Skip` prevents a burst of pings if the system was paused (e.g. laptop sleep). The default `Burst` behavior would send multiple pings immediately on wake, distorting health state.

### Timeout for individual ping

```rust
async fn ping_server(
    stdin: &mut ChildStdin,
    stdout_reader: &mut BufReader<ChildStdout>,
    id: u64,
) -> anyhow::Result<u64> {
    let start = std::time::Instant::now();
    let req = serde_json::to_string(&PingRequest { jsonrpc: "2.0", method: "ping", id })? + "\n";
    stdin.write_all(req.as_bytes()).await?;
    stdin.flush().await?;

    // 5-second timeout per HLTH-04 / D-13
    let line = tokio::time::timeout(
        Duration::from_secs(5),
        stdout_reader.next_line(),
    ).await??;  // outer ? = timeout, inner ? = IO error

    let response: JsonRpcResponse = serde_json::from_str(&line.unwrap_or_default())?;
    if response.id != id { anyhow::bail!("ID mismatch"); }
    if response.error.is_some() { anyhow::bail!("Ping returned error"); }

    Ok(start.elapsed().as_millis() as u64)
}
```

### One task per server, independent timeouts

Each server's health check runs in its own `tokio::spawn`. A slow server (5s timeout) does not delay any other server's tick. This directly satisfies HLTH-04.

---

## 4. Stdout Ownership Handoff — Phase 1 → Phase 2

### Current Phase 1 code

In `supervisor.rs`, `run_server_supervisor` does:

```rust
// Drain stdout in Phase 1 to prevent pipe-buffer backpressure (PITFALL #2).
// Phase 3 will hand `stdout` to the MCP client instead of draining here.
if let Some(stdout) = spawned.stdout {
    let drain_name = name.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(server = %drain_name, "[stdout] {}", line);
        }
    });
}
```

The comment explicitly marks this as the Phase 2 handoff point.

### What Phase 2 changes

`spawn_server` in `supervisor.rs` currently returns `SpawnedProcess { child, pid, stdout }`. Phase 2 adds `stdin`:

```rust
pub struct SpawnedProcess {
    pub child: tokio::process::Child,
    pub pid: u32,
    pub stdout: Option<ChildStdout>,
    pub stdin: Option<ChildStdin>,  // NEW in Phase 2
}
```

In `run_server_supervisor`, instead of spawning the silent drain task, Phase 2 passes both `stdin` and `stdout` to `run_health_check_loop`. The supervisor must also reset `HealthStatus` to `Unknown` each time it re-spawns the process (D-04). This means the health watch channel sender must be passed into `run_server_supervisor` (or the supervisor creates it and exposes the receiver via `ServerHandle`).

### Where the MCP client task lives relative to the supervisor task

Option A: Health check task is a child task spawned from inside `run_server_supervisor`. When the server crashes and respawns, the supervisor cancels the old health task (via a sub-`CancellationToken`) and spawns a new one with the new stdin/stdout handles.

Option B: The supervisor sends stdin/stdout handles over a channel to a separate health coordinator task.

**Recommendation: Option A.** It keeps health lifecycle coupled to process lifecycle (correct by design — health state is meaningless if the process is not running). The supervisor already uses `CancellationToken` for shutdown; a per-spawn child token handles the health task.

---

## 5. Log Aggregator — Ring Buffer Design

### VecDeque vs crossbeam bounded channel — D-05 decision

**Decision D-05 specifies `VecDeque<LogLine>` as the ring buffer.** This is correct for the use case:

- VecDeque allows O(1) push_back and pop_front. When buffer is full: `if buf.len() >= capacity { buf.pop_front(); } buf.push_back(line);`
- Crossbeam bounded channel is optimized for producer-consumer transfer, not for random-access replay. For `mcp-hub logs` history dump (reading the last N lines), VecDeque is the right structure.
- The VecDeque needs a lock for concurrent access. `tokio::sync::Mutex<VecDeque<LogLine>>` is appropriate — the mutex is held briefly per line write, never across await points.

```rust
// src/logs.rs
pub struct LogBuffer {
    lines: tokio::sync::Mutex<VecDeque<LogLine>>,
    capacity: usize,
    broadcast_tx: tokio::sync::broadcast::Sender<LogLine>,
}

impl LogBuffer {
    pub async fn push(&self, line: LogLine) {
        {
            let mut buf = self.lines.lock().await;
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        } // lock released before broadcast
        let _ = self.broadcast_tx.send(line);
    }

    pub async fn snapshot(&self) -> Vec<LogLine> {
        self.lines.lock().await.iter().cloned().collect()
    }
}
```

### tokio::sync::broadcast for streaming

`broadcast::channel(capacity)` where capacity is the broadcast channel depth (not the ring buffer size). Use 1024 as the broadcast channel capacity — this is the in-flight window, not the history. When `mcp-hub logs --follow` is implemented (Phase 3 daemon mode) it subscribes to the broadcast receiver.

**Lagged receiver handling:** `broadcast::Receiver::recv()` returns `Err(RecvError::Lagged(n))` when the receiver is too slow and misses `n` messages. Callers of the streaming API must handle this gracefully — print a "missed N messages" notice and continue. This prevents a slow terminal from blocking the aggregator.

**Phase 2 scope for log streaming:** D-06 clarifies that `mcp-hub logs --follow` in Phase 2 only dumps the ring buffer snapshot. Live `--follow` streaming requires the daemon socket (Phase 3). So the broadcast channel is set up in Phase 2 but only consumed internally (and used to drive the colored prefix output to the foreground terminal).

### Stderr interception — replacing the current drain task

Current Phase 1 supervisor:

```rust
if let Some(stderr) = child.stderr.take() {
    let server_name = name.to_string();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(server = %server_name, "{}", line);
        }
    });
}
```

Phase 2 replaces this with a task that sends each line to `LogBuffer::push`. The `Arc<LogBuffer>` is passed into the supervisor (or into a separate log aggregator struct that wraps a `HashMap<String, Arc<LogBuffer>>`).

### Per-server vs global buffer

Architecture decision: one `LogBuffer` per server (10,000 lines each), all accessible via a `LogAggregator` struct keyed by server name:

```rust
pub struct LogAggregator {
    buffers: HashMap<String, Arc<LogBuffer>>,
    // Single broadcast for "all servers" stream (interleaved)
    all_tx: broadcast::Sender<LogLine>,
}
```

The `all_tx` channel receives every line from every server, enabling `mcp-hub logs` (no filter) without merging per-server broadcasts.

---

## 6. LogLine Format — D-07 Color Prefix Style

### Format

```
mcp-github | 2026-04-02T10:15:30Z Server started on port 3000
```

Fields:
- Server name (colored, deterministic per server)
- Pipe separator ` | `
- Timestamp (UTC, ISO 8601 second precision — milliseconds add noise for human reading)
- Message from stderr line

```rust
pub struct LogLine {
    pub server: String,
    pub timestamp: std::time::SystemTime,  // or chrono::DateTime<Utc> if chrono is added
    pub message: String,
}
```

**Timestamp crate choice:** `std::time::SystemTime` avoids a new dependency for Phase 2. Format as RFC 3339 using a small helper. `chrono` is only needed if Phase 4 (Web UI) requires it — defer the dependency.

### Color assignment per server name — deterministic hash

Docker-compose style: each service gets a stable color so colors are consistent across restarts. Algorithm:

```rust
fn server_color(name: &str) -> owo_colors::AnsiColors {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let h = hasher.finish();
    // 6 distinguishable terminal colors (avoid dark colors that blend with bg)
    let palette = [
        owo_colors::AnsiColors::Cyan,
        owo_colors::AnsiColors::Green,
        owo_colors::AnsiColors::Yellow,
        owo_colors::AnsiColors::Magenta,
        owo_colors::AnsiColors::Blue,
        owo_colors::AnsiColors::BrightCyan,
    ];
    palette[(h % palette.len() as u64) as usize]
}
```

`owo-colors` is already in `Cargo.toml`. `DefaultHasher` is not stable across Rust versions but is fine for a cosmetic feature (color is not persisted). `FxHasher` or `AHash` would be more stable but add a dependency.

---

## 7. Status Table Enhancement — D-09, D-10

### New columns per D-09

Current: Name | State | PID
Phase 2: Name | Process State | Health | PID | Uptime | Restarts | Transport

### Uptime format — D-10

`HH:MM:SS` from `Instant::elapsed()` on `started_at` (already captured in `run_server_supervisor` as `let started_at = std::time::Instant::now()`).

```rust
fn format_uptime(elapsed: std::time::Duration) -> String {
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
```

`Instant` is not `Clone` and cannot be sent across task boundaries. To expose uptime to the output layer, store `uptime_since: Option<std::time::Instant>` in the state snapshot sent via the watch channel. The output layer computes elapsed time at render time.

### Updated watch channel payload

Currently: `watch::Sender<(ProcessState, Option<u32>)>`

Phase 2 change: Replace with a `ServerSnapshot` struct so new fields don't require changing all call sites:

```rust
pub struct ServerSnapshot {
    pub process_state: ProcessState,
    pub health: HealthStatus,
    pub pid: Option<u32>,
    pub uptime_since: Option<std::time::Instant>,
    pub restart_count: u32,
    pub transport: String,
}
```

This is a clean breaking change within Phase 2 — all consumers of `state_rx` are in `output.rs` and `main.rs`, both easy to update.

### comfy-table column addition

`comfy-table` supports adding columns to the header call and cells to each row call. The existing `print_status_table` signature changes to accept `&[ServerSnapshot]`. The function signature becomes:

```rust
pub fn print_status_table(servers: &[(String, ServerSnapshot)], color: bool)
```

---

## 8. `mcp-hub logs` CLI Subcommand — D-06

### Clap design

Phase 2 adds `Commands::Logs(LogsArgs)` to `cli.rs`:

```rust
#[derive(Debug, clap::Args)]
pub struct LogsArgs {
    /// Follow log output (stream new lines as they arrive). Phase 3 only in daemon mode.
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Filter to a specific server name.
    #[arg(long, short = 's', value_name = "NAME")]
    pub server: Option<String>,

    /// Number of lines to show from history (default: 100).
    #[arg(long, short = 'n', default_value = "100")]
    pub lines: usize,
}
```

### Phase 2 behavior (foreground mode)

In foreground mode, `mcp-hub logs` as a **separate process invocation** cannot access the running hub's in-memory ring buffer (no daemon socket yet). Options:

1. Print a clear "requires daemon mode (Phase 3)" message — honest and simple.
2. Dump a log file written to disk (deferred to v2/ADV-04 in REQUIREMENTS.md).

**D-06 says:** "For Phase 2, it dumps the ring buffer history from disk/state." This implies the hub writes ring buffer snapshots to a file (e.g., `~/.mcp-hub/logs/<server>.log`). The `mcp-hub logs` subcommand reads these files.

However, log file persistence is listed as ADV-04 (v2). A simpler Phase 2 approach that satisfies D-06 without contradicting it:

- When `mcp-hub start` is running in foreground mode, the `logs` stdin command (typed into the foreground hub's terminal) dumps the ring buffer.
- The CLI subcommand `mcp-hub logs` (separate invocation) prints "daemon mode required for log access from a separate process (Phase 3)."
- The live colored log output is printed to the foreground terminal as lines arrive (the "unified log stream" visible in foreground mode, satisfying LOG-01 in spirit).

This is consistent with D-11 which similarly notes that `mcp-hub status` as a separate invocation needs Phase 3.

### Foreground terminal log output

In foreground mode, each line captured by the log aggregator is immediately printed to the terminal with the colored prefix. This is the primary Phase 2 log feature — the `mcp-hub logs --follow` streaming to the foreground terminal happens naturally as lines arrive.

The foreground stdin command handler gains:
- `logs` — dump ring buffer snapshot (all servers) with colored prefixes
- `logs <name>` — dump ring buffer for specific server

---

## 9. Integration Architecture — How the Pieces Wire Together

### New structs and modules

```
src/
  types.rs          — add HealthStatus enum, ServerSnapshot struct
  logs.rs           — NEW: LogBuffer, LogLine, LogAggregator
  supervisor.rs     — extend SpawnedProcess with stdin field, add health_tx to ServerHandle
  output.rs         — extend print_status_table with new columns
  cli.rs            — add Logs(LogsArgs), Status subcommands
  main.rs           — wire LogAggregator, extend foreground loop
  mcp/
    mod.rs          — NEW module
    protocol.rs     — NEW: PingRequest, JsonRpcResponse
    health.rs       — NEW: run_health_check_loop, ping_server
```

### Startup sequence changes

```
start_all_servers()
  → for each server:
      spawn_server() → SpawnedProcess { child, pid, stdout, stdin }  // stdin added
      create watch::channel for ServerSnapshot
      create LogBuffer per server and register in LogAggregator
      spawn stderr drain task → LogAggregator::push()
      spawn health check task (owns stdin + stdout) → updates health_tx
      store ServerHandle { name, snapshot_rx, cmd_tx, task }
```

The `Arc<LogAggregator>` is created in `main.rs` before `start_all_servers` and passed in. `ServerHandle` gains a `health_tx: watch::Sender<HealthStatus>` (or the combined `snapshot_tx: watch::Sender<ServerSnapshot>`).

### Arc sharing pattern

`Arc<LogAggregator>` is cloned once per server for the stderr drain task. The main loop also holds an `Arc<LogAggregator>` for the `logs` stdin command dump.

```rust
let log_agg = Arc::new(LogAggregator::new(&config));
let mut handles = start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;
```

---

## 10. Critical Decisions Left for Planner

These are within "Claude's Discretion" from 02-CONTEXT.md. The planner should make a specific call for each:

### A. Combined vs. separate watch channels

**Recommendation: single `watch::Sender<ServerSnapshot>` per server.** Avoids multiple borrow issues when the output layer needs both process state and health together. The supervisor task owns the sender and updates it on both process state changes and health changes. Health task communicates back to the supervisor via a small `mpsc` channel, or health task directly updates a shared `Arc<Mutex<HealthStatus>>` that the supervisor reads when broadcasting the snapshot.

Simplest: health task has its own `watch::Sender<HealthStatus>`, supervisor has its own `watch::Sender<ProcessState>`, and `ServerHandle` exposes both. The output layer borrows both. Less clean but no cross-task ownership issues.

### B. Restart count tracking

`restart_count: u32` must be incremented each time the supervisor loops back to Starting. Currently `consecutive_failures` tracks this but resets on stable run. A separate `total_restarts: u32` (never resets) is what the status table should show. Add this to `run_server_supervisor` state.

### C. `uptime_since` reset timing

Reset `uptime_since` to `Some(Instant::now())` at `ProcessState::Running` transition. Set to `None` when state transitions to Stopping/Stopped/Backoff/Fatal.

### D. Log aggregator passed into `start_all_servers` vs. built separately

Prefer passing `Arc<LogAggregator>` into `start_all_servers`. This keeps supervisor.rs testable (can inject a mock log aggregator in tests).

### E. JSON-RPC ID management for Phase 2

Simple: a `u64` counter per health check loop, starting at 1, incrementing by 1. No global ID space needed in Phase 2 since requests are sequential (send ping, wait for response, send next ping). The full dispatcher pattern (Pitfall #7) is for Phase 3 concurrent introspection.

---

## 11. Pitfall Reminders for This Phase

From PITFALLS.md, specifically relevant to Phase 2:

- **Pitfall #2 (pipe blocking):** The stdout drain task is replaced by the health check loop. The loop must always read stdout, even if not waiting for a ping response (e.g., if the server sends unsolicited messages). Add a fallback drain path for non-ping lines.
- **Pitfall #4 (conflating health with liveness):** This is the central concern. The HealthStatus / ProcessState separation (D-01) directly addresses this. Never display `running` as equivalent to `healthy` in the status table.
- **Pitfall #7 (JSON-RPC ID correlation):** In Phase 2, pings are sequential so strict correlation checking (verify `response.id == request.id`) is sufficient. Note this in code comments so Phase 3 knows to upgrade to the full dispatcher.
- **Pitfall #11 (stale state / mutex contention):** Even without the web UI, the status table rendering in the foreground loop must not hold the watch channel borrow across any await points. `handle.state_rx.borrow().clone()` (already the pattern in Phase 1) is correct.
- **Pitfall #6 (blocking I/O):** VecDeque operations are sync. The `tokio::sync::Mutex` ensures they do not block the runtime — the lock is never held across an `await` in the push path (see the LogBuffer::push implementation above where the lock is released before `broadcast_tx.send`).

---

## 12. New Cargo.toml Dependencies

No new dependencies are required for Phase 2. All needed crates are already present:

| Need | Crate | Already in Cargo.toml |
|------|-------|----------------------|
| Async I/O for MCP stdin/stdout | `tokio` | Yes |
| JSON-RPC serialization | `serde_json` | Yes |
| Ring buffer | `std::collections::VecDeque` | stdlib |
| Broadcast channel | `tokio::sync::broadcast` | Part of tokio |
| Watch channel | `tokio::sync::watch` | Part of tokio |
| Color output | `owo-colors` | Yes |
| Status table | `comfy-table` | Yes |
| Timestamps | `std::time::SystemTime` | stdlib |
| Hashing for color assignment | `std::collections::hash_map::DefaultHasher` | stdlib |

`chrono` is NOT needed in Phase 2. SystemTime + manual formatting is sufficient for log line timestamps.

---

## 13. Test Strategy

### Unit tests

- `HealthStatus` state machine transitions: 7 cases (Unknown→Healthy, Healthy→Degraded at 2 misses, Degraded→Failed at 7 total, Degraded→Healthy on recovery, Failed→Healthy, reset on restart, each transition is 1 test)
- `compute_health_status(misses, current)` pure function — easy to unit test
- `format_uptime(Duration)` — test 0s, 59s, 3600s, 3661s
- `server_color(name)` — test same name → same color, different names → may differ
- `LogBuffer::push` capacity enforcement — push capacity+1 lines, assert len == capacity
- `LogBuffer::snapshot` ordering — FIFO order preserved

### Integration tests (assert_cmd)

- Start a fake MCP server (a small shell script that responds to ping) and verify `HealthStatus` transitions to Healthy
- Start a fake server that ignores stdin and verify it degrades after 2 missed pings (use short test interval)
- Verify status table output includes Health and Uptime columns
- Verify log lines appear with colored prefix in stdout

### Test fixture: minimal MCP ping responder

```bash
#!/bin/bash
# tests/fixtures/mcp-echo-server.sh
while IFS= read -r line; do
    id=$(echo "$line" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
    echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
done
```

Or a small Rust binary in `tests/fixtures/` that reads stdin JSON-RPC and responds to pings. A Rust fixture is more portable and reliable than a bash script.

---

## RESEARCH COMPLETE
