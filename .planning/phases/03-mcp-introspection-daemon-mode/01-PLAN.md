---
plan_id: "03-01"
title: "MCP Protocol Types + Dispatcher Refactor"
phase: 3
wave: 1
depends_on: []
files_modified:
  - src/mcp/protocol.rs
  - src/mcp/dispatcher.rs
  - src/mcp/health.rs
  - src/mcp/mod.rs
  - src/types.rs
  - src/supervisor.rs
requirements_addressed:
  - MCP-02
  - MCP-04
autonomous: true
---

# Plan 03-01: MCP Protocol Types + Dispatcher Refactor

<objective>
Extend protocol.rs with all MCP introspection request/response types (initialize, tools/list,
resources/list, prompts/list, notifications/initialized). Build the shared reader_task + dispatcher
pattern that replaces the current per-ping read-in-place approach. Refactor health.rs to use the
dispatcher (stdin via Arc<Mutex>, responses via oneshot channels). Add McpCapabilities and related
types to types.rs. This plan lays the foundation for all concurrent JSON-RPC communication in
Phase 3.
</objective>

---

## Task 1: Add MCP introspection types to protocol.rs

<task id="03-01-01">
<read_first>
- src/mcp/protocol.rs (PingRequest, JsonRpcResponse — current state)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 1 — all message shapes)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-04 — stdio transport only)
</read_first>

<action>
In `src/mcp/protocol.rs`:

1. **Remove the `#![allow(dead_code)]`** at the top (these types will be used).

2. **Add a generic `JsonRpcRequest` struct** for all outgoing requests:
   ```rust
   #[derive(Debug, Serialize)]
   pub struct JsonRpcRequest<T: Serialize> {
       pub jsonrpc: &'static str,
       pub method: &'static str,
       pub id: u64,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub params: Option<T>,
   }
   ```

3. **Add a `JsonRpcNotification` struct** for fire-and-forget messages (no `id`):
   ```rust
   #[derive(Debug, Serialize)]
   pub struct JsonRpcNotification {
       pub jsonrpc: &'static str,
       pub method: &'static str,
   }

   impl JsonRpcNotification {
       pub fn initialized() -> Self {
           Self {
               jsonrpc: "2.0",
               method: "notifications/initialized",
           }
       }
   }
   ```

4. **Add `InitializeParams` and `ClientInfo`** for the initialize request:
   ```rust
   #[derive(Debug, Serialize)]
   pub struct InitializeParams {
       #[serde(rename = "protocolVersion")]
       pub protocol_version: String,
       pub capabilities: serde_json::Value,
       #[serde(rename = "clientInfo")]
       pub client_info: ClientInfo,
   }

   #[derive(Debug, Serialize)]
   pub struct ClientInfo {
       pub name: String,
       pub version: String,
   }
   ```

5. **Add response result types** for deserialization from `JsonRpcResponse.result`:
   ```rust
   #[derive(Debug, Deserialize)]
   pub struct InitializeResult {
       #[serde(rename = "protocolVersion")]
       pub protocol_version: String,
       pub capabilities: ServerCapabilities,
       #[serde(rename = "serverInfo")]
       pub server_info: Option<ServerInfo>,
   }

   #[derive(Debug, Deserialize)]
   pub struct ServerCapabilities {
       pub tools: Option<serde_json::Value>,
       pub resources: Option<serde_json::Value>,
       pub prompts: Option<serde_json::Value>,
   }

   #[derive(Debug, Deserialize)]
   pub struct ServerInfo {
       pub name: String,
       pub version: Option<String>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct ToolsListResult {
       pub tools: Vec<McpTool>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct McpTool {
       pub name: String,
       pub description: Option<String>,
       #[serde(rename = "inputSchema")]
       pub input_schema: Option<serde_json::Value>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct ResourcesListResult {
       pub resources: Vec<McpResource>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct McpResource {
       pub uri: String,
       pub name: String,
       pub description: Option<String>,
       #[serde(rename = "mimeType")]
       pub mime_type: Option<String>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct PromptsListResult {
       pub prompts: Vec<McpPrompt>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct McpPrompt {
       pub name: String,
       pub description: Option<String>,
       pub arguments: Option<Vec<PromptArgument>>,
   }

   #[derive(Debug, Clone, Deserialize, Serialize)]
   pub struct PromptArgument {
       pub name: String,
       pub description: Option<String>,
       pub required: Option<bool>,
   }
   ```

6. **Add constructor helpers** for building requests:
   ```rust
   pub fn initialize_request(id: u64) -> JsonRpcRequest<InitializeParams> {
       JsonRpcRequest {
           jsonrpc: "2.0",
           method: "initialize",
           id,
           params: Some(InitializeParams {
               protocol_version: "2024-11-05".to_string(),
               capabilities: serde_json::json!({}),
               client_info: ClientInfo {
                   name: "mcp-hub".to_string(),
                   version: env!("CARGO_PKG_VERSION").to_string(),
               },
           }),
       }
   }

   pub fn tools_list_request(id: u64) -> JsonRpcRequest<()> {
       JsonRpcRequest {
           jsonrpc: "2.0",
           method: "tools/list",
           id,
           params: None,
       }
   }

   pub fn resources_list_request(id: u64) -> JsonRpcRequest<()> {
       JsonRpcRequest {
           jsonrpc: "2.0",
           method: "resources/list",
           id,
           params: None,
       }
   }

   pub fn prompts_list_request(id: u64) -> JsonRpcRequest<()> {
       JsonRpcRequest {
           jsonrpc: "2.0",
           method: "prompts/list",
           id,
           params: None,
       }
   }
   ```

7. **Keep `PingRequest` as-is** — it is already used by health.rs and will be migrated in Task 3.
</action>

<acceptance_criteria>
- grep: `pub struct JsonRpcRequest` in src/mcp/protocol.rs
- grep: `pub struct JsonRpcNotification` in src/mcp/protocol.rs
- grep: `pub struct InitializeParams` in src/mcp/protocol.rs
- grep: `pub struct InitializeResult` in src/mcp/protocol.rs
- grep: `pub struct ServerCapabilities` in src/mcp/protocol.rs
- grep: `pub struct McpTool` in src/mcp/protocol.rs
- grep: `pub struct McpResource` in src/mcp/protocol.rs
- grep: `pub struct McpPrompt` in src/mcp/protocol.rs
- grep: `pub struct PromptArgument` in src/mcp/protocol.rs
- grep: `pub fn initialize_request` in src/mcp/protocol.rs
- grep: `pub fn tools_list_request` in src/mcp/protocol.rs
- grep: `pub fn resources_list_request` in src/mcp/protocol.rs
- grep: `pub fn prompts_list_request` in src/mcp/protocol.rs
- grep: `notifications/initialized` in src/mcp/protocol.rs
- No `any` / No `unwrap()` in protocol.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Add McpCapabilities and extend ServerSnapshot

<task id="03-01-02">
<read_first>
- src/types.rs (ServerSnapshot, HealthStatus — current fields)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 8 — McpCapabilities)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-03 — store in ServerSnapshot)
</read_first>

<action>
In `src/types.rs`:

1. **Remove the `#![allow(dead_code)]`** at the top.

2. **Add imports** for the MCP types from protocol.rs:
   ```rust
   use crate::mcp::protocol::{McpTool, McpResource, McpPrompt};
   ```

3. **Add `McpCapabilities` struct**:
   ```rust
   #[derive(Debug, Clone, Default)]
   pub struct McpCapabilities {
       pub tools: Vec<McpTool>,
       pub resources: Vec<McpResource>,
       pub prompts: Vec<McpPrompt>,
       pub introspected_at: Option<std::time::Instant>,
   }
   ```

4. **Add `capabilities` field to `ServerSnapshot`**:
   ```rust
   pub struct ServerSnapshot {
       pub process_state: ProcessState,
       pub health: HealthStatus,
       pub pid: Option<u32>,
       pub uptime_since: Option<Instant>,
       pub restart_count: u32,
       pub transport: String,
       pub capabilities: McpCapabilities,  // NEW
   }
   ```

5. **Update `Default` for `ServerSnapshot`** to include `capabilities: McpCapabilities::default()`.

6. **Verify the `start_all_servers` initial snapshot** in supervisor.rs also includes the new field. The spread `..ServerSnapshot::default()` should handle this, but verify.
</action>

<acceptance_criteria>
- grep: `pub struct McpCapabilities` in src/types.rs
- grep: `pub capabilities: McpCapabilities` in src/types.rs
- grep: `pub tools: Vec<McpTool>` in src/types.rs
- grep: `pub resources: Vec<McpResource>` in src/types.rs
- grep: `pub prompts: Vec<McpPrompt>` in src/types.rs
- grep: `pub introspected_at: Option<std::time::Instant>` in src/types.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Create the dispatcher module (reader_task + shared pending map)

<task id="03-01-03">
<read_first>
- src/mcp/health.rs (ping_server — current pattern to be replaced)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 2 — dispatcher pattern, Section 10 — Risk 2 reader task lifetime)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-02 — HashMap dispatcher)
</read_first>

<action>
Create `src/mcp/dispatcher.rs`:

1. **Type aliases**:
   ```rust
   use std::collections::HashMap;
   use std::sync::Arc;
   use std::sync::atomic::{AtomicU64, Ordering};

   use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
   use tokio::process::{ChildStdin, ChildStdout};
   use tokio::sync::{Mutex, oneshot};

   use crate::mcp::protocol::JsonRpcResponse;

   pub type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>;
   pub type SharedStdin = Arc<Mutex<ChildStdin>>;
   ```

2. **Atomic ID allocator** (per-server instance, not global):
   ```rust
   pub struct IdAllocator {
       counter: AtomicU64,
   }

   impl IdAllocator {
       pub fn new() -> Self {
           Self {
               counter: AtomicU64::new(1),
           }
       }

       pub fn next_id(&self) -> u64 {
           self.counter.fetch_add(1, Ordering::Relaxed)
       }
   }
   ```

3. **`reader_task`** — the single-owner stdout reader:
   ```rust
   pub async fn reader_task(
       stdout: ChildStdout,
       pending: PendingMap,
   ) {
       let reader = BufReader::new(stdout);
       let mut lines = reader.lines();

       while let Ok(Some(line)) = lines.next_line().await {
           // Try to parse as JSON-RPC response.
           match serde_json::from_str::<JsonRpcResponse>(&line) {
               Ok(response) => {
                   let mut map = pending.lock().await;
                   if let Some(sender) = map.remove(&response.id) {
                       let _ = sender.send(response);
                   } else {
                       tracing::debug!(
                           id = response.id,
                           "Received response with no pending waiter"
                       );
                   }
               }
               Err(_) => {
                   // Non-JSON-RPC line (notification, log, etc.) — discard.
                   tracing::debug!("Non-JSON-RPC stdout line (discarded)");
               }
           }
       }

       // stdout closed — drain all pending waiters so they get RecvError.
       let mut map = pending.lock().await;
       map.drain();
       // Dropping the senders without sending causes Receivers to get RecvError.
       tracing::debug!("Reader task exiting — stdout closed, drained pending map");
   }
   ```

4. **`send_request`** — generic helper to write a request and await its response:
   ```rust
   use std::time::Duration;

   pub async fn send_request<T: serde::Serialize>(
       stdin: &SharedStdin,
       pending: &PendingMap,
       id: u64,
       request: &T,
       timeout_secs: u64,
   ) -> anyhow::Result<JsonRpcResponse> {
       let (tx, rx) = oneshot::channel();

       // Register the waiter before writing the request.
       {
           let mut map = pending.lock().await;
           map.insert(id, tx);
       }

       // Serialize and write.
       let mut json = serde_json::to_string(request)
           .map_err(|e| anyhow::anyhow!("Failed to serialize request: {e}"))?;
       json.push('\n');

       {
           let mut stdin_lock = stdin.lock().await;
           stdin_lock
               .write_all(json.as_bytes())
               .await
               .map_err(|e| anyhow::anyhow!("Failed to write to stdin: {e}"))?;
           stdin_lock
               .flush()
               .await
               .map_err(|e| anyhow::anyhow!("Failed to flush stdin: {e}"))?;
       }

       // Await the response with timeout.
       let result = tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await;

       match result {
           Ok(Ok(response)) => Ok(response),
           Ok(Err(_)) => {
               // oneshot sender was dropped (reader task exited).
               anyhow::bail!("Reader task closed before response for id={id}")
           }
           Err(_) => {
               // Timeout — remove from pending map.
               let mut map = pending.lock().await;
               map.remove(&id);
               anyhow::bail!("Request id={id} timed out after {timeout_secs}s")
           }
       }
   }
   ```

5. **`send_notification`** — fire-and-forget write to stdin (no id, no response):
   ```rust
   pub async fn send_notification<T: serde::Serialize>(
       stdin: &SharedStdin,
       notification: &T,
   ) -> anyhow::Result<()> {
       let mut json = serde_json::to_string(notification)
           .map_err(|e| anyhow::anyhow!("Failed to serialize notification: {e}"))?;
       json.push('\n');

       let mut stdin_lock = stdin.lock().await;
       stdin_lock
           .write_all(json.as_bytes())
           .await
           .map_err(|e| anyhow::anyhow!("Failed to write notification to stdin: {e}"))?;
       stdin_lock
           .flush()
           .await
           .map_err(|e| anyhow::anyhow!("Failed to flush stdin: {e}"))?;

       Ok(())
   }
   ```

6. **Update `src/mcp/mod.rs`**:
   ```rust
   pub mod dispatcher;
   pub mod health;
   pub mod protocol;
   ```
</action>

<acceptance_criteria>
- grep: `pub type PendingMap` in src/mcp/dispatcher.rs
- grep: `pub type SharedStdin` in src/mcp/dispatcher.rs
- grep: `pub struct IdAllocator` in src/mcp/dispatcher.rs
- grep: `pub async fn reader_task` in src/mcp/dispatcher.rs
- grep: `pub async fn send_request` in src/mcp/dispatcher.rs
- grep: `pub async fn send_notification` in src/mcp/dispatcher.rs
- grep: `map.drain()` in src/mcp/dispatcher.rs
- grep: `pub mod dispatcher` in src/mcp/mod.rs
- No `unwrap()` in src/mcp/dispatcher.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Refactor health.rs to use the dispatcher

<task id="03-01-04">
<read_first>
- src/mcp/health.rs (ping_server, run_health_check_loop — current implementation)
- src/mcp/dispatcher.rs (send_request, SharedStdin, PendingMap, IdAllocator — from Task 3)
- src/supervisor.rs (lines 300-335 — health task spawning, spawned.stdin/stdout handoff)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 2 — impact on health.rs)
</read_first>

<action>
1. **Rewrite `ping_server`** in `src/mcp/health.rs` to use the dispatcher:
   ```rust
   pub async fn ping_server(
       stdin: &SharedStdin,
       pending: &PendingMap,
       id: u64,
   ) -> anyhow::Result<u64> {
       let request = PingRequest::new(id);
       let start = std::time::Instant::now();

       let response = crate::mcp::dispatcher::send_request(
           stdin, pending, id, &request, 5,
       ).await?;

       if response.error.is_some() {
           anyhow::bail!(
               "Server returned error response to ping id={id}: {:?}",
               response.error
           );
       }

       Ok(start.elapsed().as_millis() as u64)
   }
   ```

2. **Rewrite `run_health_check_loop` signature** to accept shared dispatcher resources:
   ```rust
   pub async fn run_health_check_loop(
       server_name: String,
       interval_secs: u64,
       stdin: SharedStdin,
       pending: PendingMap,
       id_alloc: Arc<IdAllocator>,
       snapshot_tx: tokio::sync::watch::Sender<ServerSnapshot>,
       cancel: CancellationToken,
   )
   ```
   - Replace the owned `ChildStdin` and `ChildStdout` params with `SharedStdin` and `PendingMap`.
   - Replace the local `request_id` counter with `id_alloc.next_id()` per tick.
   - Remove the `BufReader`/`lines` creation (reader_task owns stdout now).
   - Keep the interval tick + cancellation select! pattern.
   - Keep the consecutive_misses tracking and health status computation unchanged.

3. **Update imports** in health.rs:
   - Add: `use crate::mcp::dispatcher::{SharedStdin, PendingMap, IdAllocator};`
   - Remove: `use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};`
   - Remove: `use tokio::process::{ChildStdin, ChildStdout};`
   - Add: `use std::sync::Arc;`
</action>

<acceptance_criteria>
- grep: `stdin: &SharedStdin` in src/mcp/health.rs (ping_server)
- grep: `pending: &PendingMap` in src/mcp/health.rs (ping_server)
- grep: `send_request` in src/mcp/health.rs
- grep: `stdin: SharedStdin` in src/mcp/health.rs (run_health_check_loop)
- grep: `pending: PendingMap` in src/mcp/health.rs (run_health_check_loop)
- grep: `id_alloc: Arc<IdAllocator>` in src/mcp/health.rs
- No `BufReader<ChildStdout>` in src/mcp/health.rs
- No `stdout_reader` in src/mcp/health.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 5: Wire dispatcher into supervisor.rs

<task id="03-01-05">
<read_first>
- src/supervisor.rs (run_server_supervisor — health task spawning at lines 300-335, start_all_servers)
- src/mcp/dispatcher.rs (reader_task, PendingMap, SharedStdin, IdAllocator — from Task 3)
- src/mcp/health.rs (updated run_health_check_loop signature — from Task 4)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 3 — coordination flow)
</read_first>

<action>
In `src/supervisor.rs`, modify `run_server_supervisor`:

1. **After spawning the process** (where `spawned_stdin` and `spawned_stdout` are extracted), set up the shared dispatcher:
   ```rust
   if let (Some(stdin), Some(stdout)) = (spawned_stdin, spawned_stdout) {
       let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
       let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
       let id_alloc = Arc::new(IdAllocator::new());

       // Spawn the reader task that owns stdout.
       let pending_clone = Arc::clone(&pending);
       tokio::spawn(async move {
           crate::mcp::dispatcher::reader_task(stdout, pending_clone).await;
       });

       // Spawn the health check loop using shared stdin + pending.
       let health_name = name.clone();
       let health_tx = state_tx.clone();
       let interval = config
           .health_check_interval
           .unwrap_or(crate::mcp::health::DEFAULT_HEALTH_CHECK_INTERVAL_SECS);
       let cancel = health_cancel.clone();
       let stdin_health = Arc::clone(&stdin_shared);
       let pending_health = Arc::clone(&pending);
       let id_health = Arc::clone(&id_alloc);
       tokio::spawn(async move {
           crate::mcp::health::run_health_check_loop(
               health_name,
               interval,
               stdin_health,
               pending_health,
               id_health,
               health_tx,
               cancel,
           )
           .await;
       });
   }
   ```

2. **Add imports** at the top of supervisor.rs:
   ```rust
   use crate::mcp::dispatcher::{SharedStdin, PendingMap, IdAllocator};
   use tokio::sync::Mutex;
   ```
   Note: `std::sync::Arc` should already be imported.

3. **Remove the old health loop spawning code** that passed owned `ChildStdin` and `ChildStdout` directly.

4. **Ensure `health_cancel.cancel()`** is called in all exit paths before `shutdown_process` (this should already be the case from Phase 2, verify).
</action>

<acceptance_criteria>
- grep: `SharedStdin` in src/supervisor.rs
- grep: `PendingMap` in src/supervisor.rs
- grep: `IdAllocator` in src/supervisor.rs
- grep: `reader_task` in src/supervisor.rs
- grep: `Arc::new(Mutex::new(stdin))` in src/supervisor.rs
- No `ChildStdin` passed directly to `run_health_check_loop` in src/supervisor.rs
- No `ChildStdout` passed directly to `run_health_check_loop` in src/supervisor.rs
- cargo build succeeds
- cargo clippy -- -D warnings passes
</acceptance_criteria>
</task>

---

## Task 6: Update existing tests for dispatcher refactor

<task id="03-01-06">
<read_first>
- tests/health_monitor.rs (existing health tests from Phase 2)
- src/mcp/dispatcher.rs (send_request, reader_task — from Task 3)
- src/mcp/protocol.rs (new types — from Task 1)
</read_first>

<action>
1. **Update `tests/health_monitor.rs`** to work with the new dispatcher-based ping:
   - Tests that directly called `ping_server` with `ChildStdin`/`ChildStdout` must now:
     a. Create `SharedStdin` and `PendingMap`.
     b. Spawn `reader_task` on the stdout.
     c. Call the new `ping_server` with `&stdin_shared, &pending, id`.
   - The mock responder fixtures should still work since the protocol is unchanged.

2. **Add a dispatcher unit test** in `tests/dispatcher.rs`:
   - Test that `send_request` with a matching response returns the correct `JsonRpcResponse`.
   - Test that `send_request` times out correctly when no response arrives.
   - Test that `reader_task` drains pending map when stdout closes.
   - Test concurrent `send_request` calls with different IDs resolve correctly.
   - Use `tokio::io::duplex` or spawned mock processes for stdin/stdout simulation.

3. **Add protocol serialization tests** in `tests/protocol.rs`:
   - `initialize_request` serializes to valid JSON with correct `protocolVersion`, `clientInfo.name`, `method: "initialize"`.
   - `tools_list_request` serializes to valid JSON with `method: "tools/list"`.
   - `JsonRpcNotification::initialized()` serializes to JSON without `id` field.
   - `InitializeResult` deserializes from the example JSON in the research doc.
   - `ToolsListResult` deserializes a response with 2 tools, `McpTool` fields populated.
   - `ResourcesListResult` and `PromptsListResult` deserialize correctly.

4. **Run all existing tests** to ensure nothing is broken.
</action>

<acceptance_criteria>
- grep: `SharedStdin` in tests/health_monitor.rs
- grep: `reader_task` in tests/health_monitor.rs OR tests/dispatcher.rs
- grep: `fn initialize_request_serialization` in tests/protocol.rs
- grep: `fn tools_list_result_deserialization` in tests/protocol.rs
- grep: `fn dispatcher_concurrent_requests` in tests/dispatcher.rs
- cargo test passes
- cargo clippy -- -D warnings passes
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
- [ ] protocol.rs has all 13 introspection types: JsonRpcRequest, JsonRpcNotification, InitializeParams, ClientInfo, InitializeResult, ServerCapabilities, ServerInfo, ToolsListResult, McpTool, ResourcesListResult, McpResource, PromptsListResult, McpPrompt, PromptArgument
- [ ] protocol.rs has 4 constructor functions: initialize_request, tools_list_request, resources_list_request, prompts_list_request
- [ ] types.rs has McpCapabilities struct with tools, resources, prompts, introspected_at
- [ ] ServerSnapshot has `capabilities: McpCapabilities` field
- [ ] dispatcher.rs has reader_task that routes responses by ID via PendingMap
- [ ] dispatcher.rs has send_request with timeout that uses oneshot channels
- [ ] dispatcher.rs has send_notification for fire-and-forget messages
- [ ] reader_task drains pending map on stdout close (no leaked oneshots)
- [ ] health.rs refactored: ping_server uses send_request, no direct stdout ownership
- [ ] run_health_check_loop accepts SharedStdin + PendingMap + IdAllocator
- [ ] supervisor.rs spawns reader_task + passes shared resources to health loop
- [ ] No ChildStdout passed directly to health.rs (only via reader_task in dispatcher)
- [ ] AtomicU64 ID allocator ensures unique IDs across health pings and future introspection
- [ ] All Phase 2 tests still pass (backward compat)
- [ ] New protocol serialization tests pass
- [ ] New dispatcher tests pass
- [ ] No unwrap() in production code (src/)
- [ ] cargo clippy -D warnings passes
</verification>
