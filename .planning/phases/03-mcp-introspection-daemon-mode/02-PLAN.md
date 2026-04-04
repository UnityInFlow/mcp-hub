---
plan_id: "03-02"
title: "MCP Introspection Flow"
phase: 3
wave: 2
depends_on:
  - "03-01"
files_modified:
  - src/mcp/introspect.rs
  - src/mcp/mod.rs
  - src/supervisor.rs
  - src/output.rs
  - tests/introspection.rs
requirements_addressed:
  - MCP-01
  - MCP-03
autonomous: true
---

# Plan 03-02: MCP Introspection Flow

<objective>
Implement the full MCP capability discovery sequence: initialize handshake, notifications/initialized,
concurrent tools/list + resources/list + prompts/list. Store results in ServerSnapshot via
McpCapabilities. Trigger introspection when a server first reaches Healthy status.
Update the status table to show tool/resource/prompt counts.
</objective>

---

## Task 1: Create the introspection module

<task id="03-02-01">
<read_first>
- src/mcp/protocol.rs (initialize_request, tools_list_request, resources_list_request, prompts_list_request, JsonRpcNotification::initialized, InitializeResult, ServerCapabilities, ToolsListResult, ResourcesListResult, PromptsListResult — from 03-01)
- src/mcp/dispatcher.rs (send_request, send_notification, SharedStdin, PendingMap, IdAllocator — from 03-01)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Sections 1, 2, 3 — introspection flow, concurrent requests, coordination)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-01, D-02, D-03 — introspect once on Healthy, concurrent via dispatcher, store in snapshot)
</read_first>

<action>
Create `src/mcp/introspect.rs`:

1. **`run_introspection` async function** — the main entry point:
   ```rust
   use std::sync::Arc;
   use std::time::Duration;

   use crate::mcp::dispatcher::{send_request, send_notification, SharedStdin, PendingMap, IdAllocator};
   use crate::mcp::protocol::{
       initialize_request, tools_list_request, resources_list_request,
       prompts_list_request, JsonRpcNotification, InitializeResult,
       ServerCapabilities, ToolsListResult, ResourcesListResult, PromptsListResult,
   };
   use crate::types::{McpCapabilities, ServerSnapshot};

   pub async fn run_introspection(
       server_name: &str,
       stdin: &SharedStdin,
       pending: &PendingMap,
       id_alloc: &IdAllocator,
       snapshot_tx: &tokio::sync::watch::Sender<ServerSnapshot>,
   ) -> anyhow::Result<McpCapabilities> {
       // Step 1: Send initialize request and await response.
       let init_id = id_alloc.next_id();
       let init_req = initialize_request(init_id);
       let init_response = send_request(stdin, pending, init_id, &init_req, 10).await?;

       // Parse initialize result to check server capabilities.
       let init_result: InitializeResult = match init_response.result {
           Some(value) => serde_json::from_value(value)
               .map_err(|e| anyhow::anyhow!("Failed to parse initialize result: {e}"))?,
           None => {
               if let Some(err) = init_response.error {
                   anyhow::bail!("Server returned error on initialize: {err}");
               }
               anyhow::bail!("Server returned empty initialize response");
           }
       };

       tracing::info!(
           server = %server_name,
           protocol = %init_result.protocol_version,
           server_name = ?init_result.server_info.as_ref().map(|s| &s.name),
           "MCP initialize handshake complete"
       );

       // Step 2: Send notifications/initialized (fire-and-forget).
       let notification = JsonRpcNotification::initialized();
       send_notification(stdin, &notification).await?;

       // Step 3: Send concurrent list requests based on declared capabilities.
       let caps = &init_result.capabilities;
       let capabilities = fetch_capabilities(
           server_name, stdin, pending, id_alloc, caps,
       ).await;

       // Step 4: Update the snapshot with introspected capabilities.
       let caps_clone = capabilities.clone();
       snapshot_tx.send_modify(|s| {
           s.capabilities = caps_clone;
       });

       tracing::info!(
           server = %server_name,
           tools = capabilities.tools.len(),
           resources = capabilities.resources.len(),
           prompts = capabilities.prompts.len(),
           "Introspection complete"
       );

       Ok(capabilities)
   }
   ```

2. **`fetch_capabilities` helper** — concurrent list requests:
   ```rust
   async fn fetch_capabilities(
       server_name: &str,
       stdin: &SharedStdin,
       pending: &PendingMap,
       id_alloc: &IdAllocator,
       server_caps: &ServerCapabilities,
   ) -> McpCapabilities {
       // Allocate IDs for each concurrent request.
       let tools_id = id_alloc.next_id();
       let resources_id = id_alloc.next_id();
       let prompts_id = id_alloc.next_id();

       // Build requests conditionally based on server capabilities.
       let tools_fut = if server_caps.tools.is_some() {
           let req = tools_list_request(tools_id);
           Some(send_request(stdin, pending, tools_id, &req, 10))
       } else {
           None
       };

       let resources_fut = if server_caps.resources.is_some() {
           let req = resources_list_request(resources_id);
           Some(send_request(stdin, pending, resources_id, &req, 10))
       } else {
           None
       };

       let prompts_fut = if server_caps.prompts.is_some() {
           let req = prompts_list_request(prompts_id);
           Some(send_request(stdin, pending, prompts_id, &req, 10))
       } else {
           None
       };

       // Await all three concurrently using tokio::join!
       // Wrap each in an Option future.
       let (tools_result, resources_result, prompts_result) = tokio::join!(
           async {
               match tools_fut {
                   Some(fut) => Some(fut.await),
                   None => None,
               }
           },
           async {
               match resources_fut {
                   Some(fut) => Some(fut.await),
                   None => None,
               }
           },
           async {
               match prompts_fut {
                   Some(fut) => Some(fut.await),
                   None => None,
               }
           },
       );

       // Parse results, logging errors but not failing entirely (Risk 3).
       let tools = parse_tools_result(server_name, tools_result);
       let resources = parse_resources_result(server_name, resources_result);
       let prompts = parse_prompts_result(server_name, prompts_result);

       McpCapabilities {
           tools,
           resources,
           prompts,
           introspected_at: Some(std::time::Instant::now()),
       }
   }
   ```

3. **Parser helpers** that handle errors gracefully (Risk 3 from research):
   ```rust
   fn parse_tools_result(
       server_name: &str,
       result: Option<anyhow::Result<crate::mcp::protocol::JsonRpcResponse>>,
   ) -> Vec<crate::mcp::protocol::McpTool> {
       // If None (capability not declared), return empty.
       // If Err (timeout, IO error), log warning and return empty.
       // If response has error field, log warning and return empty.
       // If response has result, deserialize ToolsListResult.
       // On deserialization error, log warning and return empty.
       // ...
   }
   ```
   Implement analogous `parse_resources_result` and `parse_prompts_result` functions.
   Every error path logs a warning with `tracing::warn!` and returns an empty Vec -- never panics or propagates.

4. **Update `src/mcp/mod.rs`**:
   ```rust
   pub mod dispatcher;
   pub mod health;
   pub mod introspect;
   pub mod protocol;
   ```
</action>

<acceptance_criteria>
- grep: `pub async fn run_introspection` in src/mcp/introspect.rs
- grep: `async fn fetch_capabilities` in src/mcp/introspect.rs
- grep: `fn parse_tools_result` in src/mcp/introspect.rs
- grep: `fn parse_resources_result` in src/mcp/introspect.rs
- grep: `fn parse_prompts_result` in src/mcp/introspect.rs
- grep: `send_notification.*initialized` in src/mcp/introspect.rs
- grep: `tokio::join!` in src/mcp/introspect.rs
- grep: `pub mod introspect` in src/mcp/mod.rs
- No `unwrap()` in src/mcp/introspect.rs
- No panic on server error responses (graceful degradation)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Trigger introspection from supervisor on first Healthy

<task id="03-02-02">
<read_first>
- src/supervisor.rs (run_server_supervisor — dispatcher wiring from 03-01-05)
- src/mcp/introspect.rs (run_introspection — from Task 1)
- src/mcp/health.rs (run_health_check_loop — updates snapshot health)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-01 — introspect once on Healthy, re-introspect on restart)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 3 — flow at server startup)
</read_first>

<action>
In `src/supervisor.rs`, modify the dispatcher wiring block inside `run_server_supervisor`:

1. **After spawning reader_task and health loop**, spawn an introspection trigger task that watches for the first `Healthy` status:
   ```rust
   // Spawn introspection trigger — watches for first Healthy, then introspects once.
   let introspect_name = name.clone();
   let stdin_introspect = Arc::clone(&stdin_shared);
   let pending_introspect = Arc::clone(&pending);
   let id_introspect = Arc::clone(&id_alloc);
   let snapshot_introspect = state_tx.clone();
   let mut health_rx = state_tx.subscribe();
   let introspect_cancel = health_cancel.clone();

   tokio::spawn(async move {
       loop {
           tokio::select! {
               _ = introspect_cancel.cancelled() => return,
               result = health_rx.changed() => {
                   if result.is_err() {
                       return; // watch channel closed
                   }
                   let snapshot = health_rx.borrow().clone();
                   if matches!(snapshot.health, crate::types::HealthStatus::Healthy { .. }) {
                       // First Healthy — run introspection.
                       match crate::mcp::introspect::run_introspection(
                           &introspect_name,
                           &stdin_introspect,
                           &pending_introspect,
                           &id_introspect,
                           &snapshot_introspect,
                       ).await {
                           Ok(_caps) => {
                               tracing::info!(
                                   server = %introspect_name,
                                   "MCP introspection succeeded"
                               );
                           }
                           Err(err) => {
                               tracing::warn!(
                                   server = %introspect_name,
                                   "MCP introspection failed: {err}"
                               );
                           }
                       }
                       return; // Only introspect once per process spawn.
                   }
               }
           }
       }
   });
   ```

2. **On restart**, the entire dispatcher infrastructure (reader_task, health loop, introspection trigger) is cancelled via `health_cancel.cancel()` and rebuilt for the new process. This ensures re-introspection happens naturally.

3. **Ensure `McpCapabilities` resets** on process restart. In the `ProcessState::Starting` send_modify block, add:
   ```rust
   s.capabilities = McpCapabilities::default();
   ```
   This clears stale capabilities from the previous process.
</action>

<acceptance_criteria>
- grep: `run_introspection` in src/supervisor.rs
- grep: `HealthStatus::Healthy` in src/supervisor.rs (introspection trigger)
- grep: `introspect_cancel.cancelled()` in src/supervisor.rs
- grep: `McpCapabilities::default()` in src/supervisor.rs (reset on Starting)
- Introspection runs only once per process spawn (return after first Healthy)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Update status table with capability counts

<task id="03-02-03">
<read_first>
- src/output.rs (format_status_table, print_status_table — current columns)
- src/types.rs (McpCapabilities — from 03-01-02)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-03 — counts available to status table)
</read_first>

<action>
In `src/output.rs`:

1. **Add a "Capabilities" column** to the status table after "Transport":
   ```rust
   table.set_header(vec![
       "Name",
       "State",
       "Health",
       "PID",
       "Uptime",
       "Restarts",
       "Transport",
       "Tools",  // NEW
   ]);
   ```

2. **Format the capabilities cell**:
   ```rust
   let caps = &snapshot.capabilities;
   let tools_str = if caps.introspected_at.is_some() {
       format!(
           "{}T/{}R/{}P",
           caps.tools.len(),
           caps.resources.len(),
           caps.prompts.len(),
       )
   } else {
       "-".to_string()
   };
   ```
   Format: `"3T/2R/1P"` for 3 tools, 2 resources, 1 prompt. `"-"` if not yet introspected.

3. **Add the cell** to each row:
   ```rust
   table.add_row(vec![
       Cell::new(name),
       state_cell,
       health_cell,
       Cell::new(&pid_str),
       Cell::new(&uptime_str),
       Cell::new(&restarts_str),
       Cell::new(&snapshot.transport),
       Cell::new(&tools_str),  // NEW
   ]);
   ```
</action>

<acceptance_criteria>
- grep: `"Tools"` in src/output.rs (header)
- grep: `introspected_at` in src/output.rs
- grep: `T/.*R/.*P` in src/output.rs (format pattern)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Introspection integration tests

<task id="03-02-04">
<read_first>
- src/mcp/introspect.rs (run_introspection — from Task 1)
- src/mcp/protocol.rs (all introspection types)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 11 — test strategy)
- tests/health_monitor.rs (existing test patterns)
</read_first>

<action>
Create `tests/introspection.rs`:

1. **Enhance the mock MCP server fixture** (or create `tests/fixtures/mock-mcp-server.sh`):
   - Extend the ping-responder script to also handle `initialize`, `tools/list`, `resources/list`, `prompts/list`.
   - On `initialize`: respond with `{"jsonrpc":"2.0","id":<id>,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{},"resources":{},"prompts":{}},"serverInfo":{"name":"mock","version":"1.0"}}}`.
   - On `tools/list`: respond with 2 tools in the result.
   - On `resources/list`: respond with 1 resource.
   - On `prompts/list`: respond with 1 prompt.
   - On `ping`: respond with `{"jsonrpc":"2.0","id":<id>,"result":{}}`.
   - Ignore `notifications/initialized` (no response expected for notifications).

2. **Test: successful introspection captures correct counts**:
   - Spawn mock MCP server.
   - Create SharedStdin, PendingMap, IdAllocator, reader_task.
   - Create a watch channel with default ServerSnapshot.
   - Call `run_introspection`.
   - Assert capabilities: tools.len() == 2, resources.len() == 1, prompts.len() == 1.
   - Assert introspected_at is Some.
   - Assert the snapshot watch channel reflects updated capabilities.

3. **Test: server missing resources capability**:
   - Mock server responds to initialize with capabilities that omit `resources`.
   - Call `run_introspection`.
   - Assert tools populated, resources empty (skipped), prompts populated.

4. **Test: server returns error on tools/list**:
   - Mock server responds to `tools/list` with `{"jsonrpc":"2.0","id":<id>,"error":{"code":-32601,"message":"Method not found"}}`.
   - Call `run_introspection`.
   - Assert tools is empty Vec, resources and prompts still populated.
   - No panic or error propagation.

5. **Test: introspection timeout**:
   - Mock server accepts initialize but never responds to list requests.
   - Call `run_introspection`.
   - Verify it completes (with warnings) within reasonable time, returning empty capabilities for timed-out methods.
</action>

<acceptance_criteria>
- grep: `fn introspection_captures_correct_counts` in tests/introspection.rs
- grep: `fn introspection_skips_unsupported_capability` in tests/introspection.rs
- grep: `fn introspection_handles_error_response` in tests/introspection.rs
- grep: `fn introspection_timeout` in tests/introspection.rs
- cargo test --test introspection passes
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
- [ ] run_introspection sends initialize, waits for response, sends notifications/initialized, then concurrent list requests
- [ ] Concurrent tools/list + resources/list + prompts/list via tokio::join! with separate IDs
- [ ] Server capabilities from initialize response respected (skip list requests for unsupported families)
- [ ] Error responses from list requests handled gracefully (warn + empty vec, no panic)
- [ ] Timeout on any individual request does not block others (10s per request)
- [ ] McpCapabilities stored in ServerSnapshot via snapshot_tx.send_modify
- [ ] Introspection triggered once per process spawn when health first reaches Healthy
- [ ] Capabilities reset to McpCapabilities::default() on process restart
- [ ] Status table shows tool/resource/prompt counts ("3T/2R/1P" format)
- [ ] All introspection tests pass
- [ ] All existing Phase 2 tests still pass
- [ ] No unwrap() in production code (src/)
- [ ] cargo clippy -D warnings passes
</verification>
