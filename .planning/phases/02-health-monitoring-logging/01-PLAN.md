---
plan_id: "02-01"
title: "Types + Log Aggregator"
phase: 2
wave: 1
depends_on: []
files_modified:
  - src/types.rs
  - src/logs.rs
  - src/mcp/mod.rs
  - src/mcp/protocol.rs
  - src/main.rs
  - tests/health_types.rs
  - tests/log_buffer.rs
requirements_addressed:
  - HLTH-03
  - LOG-03
  - LOG-04
  - LOG-05
autonomous: true
---

# Plan 02-01: Types + Log Aggregator

<objective>
Define the HealthStatus enum, ServerSnapshot struct, LogLine struct, and LogBuffer/LogAggregator
for the ring buffer log system. Establish the `src/mcp/` module with minimal JSON-RPC protocol
types for the ping mechanism. These are the foundational data types and infrastructure that
Plans 02-02 and 02-03 build upon.
</objective>

---

## Task 1: Add HealthStatus enum and ServerSnapshot struct to types.rs

<task id="02-01-01">
<read_first>
- src/types.rs (current ProcessState, BackoffConfig)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-01, D-02, D-03, D-04 — health state model)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 2 — HealthStatus state machine)
</read_first>

<action>
Add to `src/types.rs`:

1. **`HealthStatus` enum** (HLTH-03):
   ```rust
   #[derive(Debug, Clone, Default)]
   pub enum HealthStatus {
       #[default]
       Unknown,
       Healthy {
           latency_ms: u64,
           last_checked: std::time::Instant,
       },
       Degraded {
           consecutive_misses: u32,
           last_success: Option<std::time::Instant>,
       },
       Failed {
           consecutive_misses: u32,
       },
   }
   ```

2. **`Display` impl for `HealthStatus`**:
   - `Unknown` -> `"unknown"`
   - `Healthy { .. }` -> `"healthy"`
   - `Degraded { consecutive_misses, .. }` -> `"degraded (N missed)"`
   - `Failed { consecutive_misses }` -> `"failed (N missed)"`

3. **`ServerSnapshot` struct** to replace the `(ProcessState, Option<u32>)` watch channel payload:
   ```rust
   #[derive(Debug, Clone)]
   pub struct ServerSnapshot {
       pub process_state: ProcessState,
       pub health: HealthStatus,
       pub pid: Option<u32>,
       pub uptime_since: Option<std::time::Instant>,
       pub restart_count: u32,
       pub transport: String,
   }
   ```

4. **`ServerSnapshot::default()` impl** (or `new()` associated function):
   - `process_state: ProcessState::Stopped`
   - `health: HealthStatus::Unknown`
   - `pid: None`
   - `uptime_since: None`
   - `restart_count: 0`
   - `transport: "stdio".to_string()`

5. **`compute_health_status` pure function** for health state transitions:
   ```rust
   pub fn compute_health_status(
       consecutive_misses: u32,
       current: &HealthStatus,
   ) -> HealthStatus
   ```
   Logic per D-02/D-03:
   - 0 misses: unreachable in failure path (caller handles success separately)
   - 1 miss: stay at current state (or Unknown/Healthy -> still Healthy-ish, not yet Degraded)
   - 2+ consecutive misses AND currently Healthy/Unknown: -> `Degraded { consecutive_misses, last_success }`
   - 2..6 misses AND currently Degraded: stay Degraded with updated count
   - 7+ total consecutive misses: -> `Failed { consecutive_misses }`
   - If currently Failed: stay Failed with updated count

6. **`format_uptime` helper function**:
   ```rust
   pub fn format_uptime(elapsed: std::time::Duration) -> String
   ```
   Returns `"HH:MM:SS"` format. Handle 0s, 59s, 3600s, 3661s, >24h correctly.
</action>

<acceptance_criteria>
- grep: `pub enum HealthStatus` in src/types.rs
- grep: `pub struct ServerSnapshot` in src/types.rs
- grep: `pub fn compute_health_status` in src/types.rs
- grep: `pub fn format_uptime` in src/types.rs
- grep: `impl fmt::Display for HealthStatus` in src/types.rs
- cargo build succeeds with no warnings
</acceptance_criteria>
</task>

---

## Task 2: Create src/mcp/ module with JSON-RPC protocol types

<task id="02-01-02">
<read_first>
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Sections 1, 3 — MCP JSON-RPC ping, minimal types)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-12, D-15 — MCP ping mechanism)
- .planning/research/PITFALLS.md (Pitfall #7 — JSON-RPC ID correlation)
</read_first>

<action>
1. Create `src/mcp/mod.rs`:
   ```rust
   pub mod protocol;
   ```

2. Create `src/mcp/protocol.rs` with:
   ```rust
   use serde::{Deserialize, Serialize};

   /// JSON-RPC 2.0 ping request sent to MCP servers for health checking.
   ///
   /// Phase 2 only uses `ping`. Phase 3 will add `initialize`, `tools/list`,
   /// `resources/list`, `prompts/list` request types.
   #[derive(Debug, Serialize)]
   pub struct PingRequest {
       pub jsonrpc: &'static str,
       pub method: &'static str,
       pub id: u64,
   }

   impl PingRequest {
       pub fn new(id: u64) -> Self {
           Self {
               jsonrpc: "2.0",
               method: "ping",
               id,
           }
       }
   }

   /// Generic JSON-RPC 2.0 response.
   ///
   /// Phase 2 only validates `id` correlation and checks for `error`.
   /// Phase 3 will parse `result` for introspection data.
   #[derive(Debug, Deserialize)]
   pub struct JsonRpcResponse {
       pub id: u64,
       pub result: Option<serde_json::Value>,
       pub error: Option<serde_json::Value>,
   }
   ```

3. Register the module in `src/main.rs`: add `mod mcp;` to the module declarations.
</action>

<acceptance_criteria>
- grep: `pub struct PingRequest` in src/mcp/protocol.rs
- grep: `pub struct JsonRpcResponse` in src/mcp/protocol.rs
- grep: `pub fn new` in src/mcp/protocol.rs
- grep: `mod mcp` in src/main.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Create LogLine struct and LogBuffer with ring buffer

<task id="02-01-03">
<read_first>
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Sections 5, 6 — ring buffer design, LogLine format)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-05, D-06, D-07, D-08 — log aggregation decisions)
- src/types.rs (for reference on style)
</read_first>

<action>
Create `src/logs.rs` with:

1. **`LogLine` struct**:
   ```rust
   #[derive(Debug, Clone)]
   pub struct LogLine {
       pub server: String,
       pub timestamp: std::time::SystemTime,
       pub message: String,
   }
   ```

2. **`LogBuffer` struct** — per-server ring buffer:
   ```rust
   pub struct LogBuffer {
       lines: tokio::sync::Mutex<std::collections::VecDeque<LogLine>>,
       capacity: usize,
   }
   ```

3. **`LogBuffer` impl**:
   - `pub fn new(capacity: usize) -> Self`
   - `pub async fn push(&self, line: LogLine)` — if `len >= capacity`, pop_front before push_back
   - `pub async fn snapshot(&self) -> Vec<LogLine>` — clone all lines out
   - `pub async fn snapshot_last(&self, n: usize) -> Vec<LogLine>` — clone last N lines
   - `pub async fn len(&self) -> usize`

4. **`LogAggregator` struct** — keyed collection of per-server buffers + broadcast:
   ```rust
   pub struct LogAggregator {
       buffers: std::collections::HashMap<String, std::sync::Arc<LogBuffer>>,
       all_tx: tokio::sync::broadcast::Sender<LogLine>,
   }
   ```

5. **`LogAggregator` impl**:
   - `pub fn new(server_names: &[String], capacity_per_server: usize) -> Self` — creates one `LogBuffer` per server, creates `broadcast::channel(1024)` for `all_tx`
   - `pub async fn push(&self, server: &str, message: String)` — creates a `LogLine` with `SystemTime::now()`, pushes to the server's buffer, sends on `all_tx` (ignore lagged errors)
   - `pub fn get_buffer(&self, server: &str) -> Option<&std::sync::Arc<LogBuffer>>` — returns ref to a server's buffer
   - `pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LogLine>` — subscribe to all-server stream
   - `pub async fn snapshot_all(&self) -> Vec<LogLine>` — merge all buffers, sort by timestamp
   - `pub fn server_names(&self) -> Vec<&String>` — list registered servers

6. **`format_log_line` function** for terminal display:
   ```rust
   pub fn format_log_line(line: &LogLine, color: bool) -> String
   ```
   Format: `"server-name | 2026-04-02T10:15:30Z message"` with colored server name when `color=true`.
   Use `owo_colors` for coloring. Use deterministic hash-based color assignment from server name.

7. **`server_color` helper** (private):
   ```rust
   fn server_color(name: &str) -> owo_colors::AnsiColors
   ```
   Hash name with `DefaultHasher`, pick from a 6-color palette: Cyan, Green, Yellow, Magenta, Blue, BrightCyan.

8. **`format_system_time` helper** (private):
   Format `SystemTime` as `YYYY-MM-DDTHH:MM:SSZ` (RFC 3339, second precision) using manual arithmetic on `duration_since(UNIX_EPOCH)`. No chrono dependency.

9. Register the module in `src/main.rs`: add `mod logs;`
</action>

<acceptance_criteria>
- grep: `pub struct LogLine` in src/logs.rs
- grep: `pub struct LogBuffer` in src/logs.rs
- grep: `pub struct LogAggregator` in src/logs.rs
- grep: `pub async fn push` in src/logs.rs
- grep: `pub async fn snapshot` in src/logs.rs
- grep: `pub fn format_log_line` in src/logs.rs
- grep: `fn server_color` in src/logs.rs
- grep: `mod logs` in src/main.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Unit tests for HealthStatus transitions and format_uptime

<task id="02-01-04">
<read_first>
- src/types.rs (HealthStatus, compute_health_status, format_uptime — from Task 1)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 13 — test strategy for health transitions)
- .planning/phases/02-health-monitoring-logging/02-CONTEXT.md (D-02, D-03, D-04 — transition thresholds)
</read_first>

<action>
Create `tests/health_types.rs` with these test cases:

1. **Health state transitions** (7 cases):
   - `unknown_to_healthy_on_zero_misses` — verify `HealthStatus::Unknown` stays Unknown when no ping has happened (compute_health_status is not called on success path; test the success assignment directly)
   - `healthy_to_degraded_at_2_misses` — consecutive_misses=2, currently Healthy -> Degraded
   - `degraded_stays_degraded_at_3_to_6_misses` — consecutive_misses=3..6, currently Degraded -> still Degraded with updated count
   - `degraded_to_failed_at_7_misses` — consecutive_misses=7, currently Degraded -> Failed
   - `degraded_to_healthy_on_recovery` — after being Degraded, a success resets to Healthy (test that success path creates Healthy variant)
   - `failed_to_healthy_on_recovery` — same recovery test from Failed state
   - `health_resets_to_unknown_on_restart` — verify Default::default() is Unknown (D-04)

2. **`format_uptime` tests**:
   - `format_uptime_zero` — 0s -> `"00:00:00"`
   - `format_uptime_59_secs` — 59s -> `"00:00:59"`
   - `format_uptime_one_hour` — 3600s -> `"01:00:00"`
   - `format_uptime_mixed` — 3661s -> `"01:01:01"`
   - `format_uptime_over_24h` — 90061s -> `"25:01:01"`

3. **`HealthStatus::Display` tests**:
   - Unknown displays as `"unknown"`
   - Healthy displays as `"healthy"`
   - Degraded displays as `"degraded (3 missed)"`
   - Failed displays as `"failed (7 missed)"`
</action>

<acceptance_criteria>
- grep: `fn healthy_to_degraded_at_2_misses` in tests/health_types.rs
- grep: `fn degraded_to_failed_at_7_misses` in tests/health_types.rs
- grep: `fn format_uptime_zero` in tests/health_types.rs
- cargo test --test health_types passes (all tests green)
</acceptance_criteria>
</task>

---

## Task 5: Unit tests for LogBuffer ring buffer behavior

<task id="02-01-05">
<read_first>
- src/logs.rs (LogBuffer, LogAggregator — from Task 3)
- .planning/phases/02-health-monitoring-logging/02-RESEARCH.md (Section 13 — LogBuffer tests)
</read_first>

<action>
Create `tests/log_buffer.rs` with:

1. **Ring buffer capacity enforcement**:
   - `push_within_capacity` — push 5 lines into buffer of capacity 10, verify len() == 5
   - `push_evicts_oldest_at_capacity` — push capacity+1 lines, verify len() == capacity and oldest line is gone (first line's message not in snapshot)
   - `push_many_over_capacity` — push 2x capacity lines, verify only last `capacity` lines remain

2. **Snapshot ordering**:
   - `snapshot_preserves_fifo_order` — push A, B, C; snapshot returns [A, B, C] in order
   - `snapshot_last_returns_tail` — push 10 lines, snapshot_last(3) returns last 3

3. **LogAggregator multi-server**:
   - `aggregator_push_to_correct_server` — push lines to server "a" and "b", verify each buffer only has its own lines
   - `aggregator_snapshot_all_merges_and_sorts` — push interleaved lines with different timestamps, verify snapshot_all returns sorted by timestamp
   - `aggregator_subscribe_receives_all` — subscribe to broadcast, push lines, verify subscriber receives them

4. **format_log_line**:
   - `format_log_line_no_color` — verify output contains server name, pipe separator, timestamp, message
   - `format_log_line_with_color` — verify colored output is longer than non-colored (ANSI codes present)

5. **Edge cases**:
   - `empty_buffer_snapshot` — snapshot of empty buffer returns empty vec
   - `snapshot_last_more_than_available` — snapshot_last(100) on buffer with 3 lines returns 3 lines
</action>

<acceptance_criteria>
- grep: `fn push_evicts_oldest_at_capacity` in tests/log_buffer.rs
- grep: `fn snapshot_preserves_fifo_order` in tests/log_buffer.rs
- grep: `fn aggregator_push_to_correct_server` in tests/log_buffer.rs
- cargo test --test log_buffer passes (all tests green)
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run in sequence:

```bash
cargo build 2>&1 | head -5
cargo clippy -- -D warnings 2>&1 | head -10
cargo test --test health_types 2>&1
cargo test --test log_buffer 2>&1
cargo fmt -- --check 2>&1 | head -5
```

### must_haves
- [ ] `HealthStatus` enum with 4 variants (Unknown, Healthy, Degraded, Failed) exists in src/types.rs
- [ ] `ServerSnapshot` struct with process_state, health, pid, uptime_since, restart_count, transport fields
- [ ] `compute_health_status` implements D-02 (2 misses -> Degraded) and D-03 (7 misses -> Failed) thresholds
- [ ] `format_uptime` returns HH:MM:SS format
- [ ] `PingRequest` and `JsonRpcResponse` in src/mcp/protocol.rs with serde derives
- [ ] `LogLine` with server, timestamp, message fields
- [ ] `LogBuffer` with VecDeque ring buffer, push evicts oldest at capacity
- [ ] `LogAggregator` with per-server buffers and broadcast channel
- [ ] `format_log_line` with colored server name prefix and pipe separator
- [ ] All health type tests pass (7 transition tests + 5 format_uptime tests + 4 Display tests)
- [ ] All log buffer tests pass (capacity, ordering, multi-server, edge cases)
- [ ] cargo clippy -D warnings passes
- [ ] cargo fmt --check passes
- [ ] No unwrap() in production code (src/)
</verification>
