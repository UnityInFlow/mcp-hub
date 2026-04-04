# Plan 03-02 Summary: MCP Introspection Flow

**Status:** Complete  
**Commits:** 2  
**Tests:** 4 new, 110 total (all passing)

---

## What was implemented

### Task 1 — `src/mcp/introspect.rs` (new file)

- `pub async fn run_introspection` — full MCP capability discovery sequence:
  1. Sends `initialize` request, parses `InitializeResult`
  2. Sends `notifications/initialized` fire-and-forget
  3. Calls `fetch_capabilities` for concurrent list requests
  4. Stores results in `ServerSnapshot` via `snapshot_tx.send_modify`

- `async fn fetch_capabilities` — allocates 3 distinct IDs, then uses `tokio::join!`
  to concurrently await `tools/list`, `resources/list`, and `prompts/list`. Each
  future is self-contained in its async block so the request value lives for the
  full `await` duration (fixes borrow lifetime issue from building futures outside blocks).

- `fn parse_tools_result` / `parse_resources_result` / `parse_prompts_result` —
  graceful degradation parsers: `None` (capability not declared) returns empty Vec
  silently; `Err` (timeout/IO), error response, or deserialization failure logs
  `tracing::warn!` and returns empty Vec. No panics, no error propagation.

- `src/mcp/mod.rs` — added `pub mod introspect`.

### Task 2 — `src/supervisor.rs`

- `McpCapabilities::default()` is set in the `ProcessState::Starting` `send_modify`
  block, clearing stale capabilities from the previous process on every restart.

- After the health check loop spawn, a new tokio task watches the `state_tx` watch
  channel for the first `HealthStatus::Healthy` transition. On first Healthy:
  - Calls `crate::mcp::introspect::run_introspection`
  - Logs success or warning as appropriate
  - Returns (never introspects again for this process spawn)
  - The task is cancelled via `introspect_cancel` (child of `health_cancel`) on
    process shutdown or restart.

### Task 3 — `src/output.rs`

- Added `"Tools"` column header after `"Transport"` in `format_status_table`.
- Format: `"3T/2R/1P"` (tool count, resource count, prompt count) when
  `caps.introspected_at.is_some()`; `"-"` when not yet introspected.

### Task 4 — `tests/introspection.rs` + `tests/fixtures/mock-mcp-server.sh`

**Mock server** (`mock-mcp-server.sh`):
- Handles `initialize` (responds with full capability set by default)
- Handles `notifications/initialized` (no response — fire-and-forget)
- Handles `tools/list` (2 tools: search, fetch)
- Handles `resources/list` (1 resource: file:///data)
- Handles `prompts/list` (1 prompt: summarize)
- Handles `ping`
- Env var flags: `MOCK_SKIP_RESOURCES=1`, `MOCK_TOOLS_ERROR=1`, `MOCK_SILENT_LISTS=1`

**Tests:**
1. `introspection_captures_correct_counts` — asserts 2 tools, 1 resource, 1 prompt;
   `introspected_at` is `Some`; watch channel snapshot reflects same counts.
2. `introspection_skips_unsupported_capability` — server omits `resources` from
   `initialize` capabilities; verifies tools=2, resources=0, prompts=1.
3. `introspection_handles_error_response` — server returns JSON-RPC error on
   `tools/list`; verifies tools=0, resources=1, prompts=1; no panic.
4. `introspection_timeout` — server exits after `initialize` (stream closes);
   list requests get `RecvError`; verifies completion without hanging (<14s).

---

## Verification

```
cargo build          — 0 errors
cargo clippy -D warnings — no issues
cargo test           — 110 passed (15 suites)
cargo fmt -- --check — clean
```

---

## Acceptance criteria satisfied

- [x] `pub async fn run_introspection` in src/mcp/introspect.rs
- [x] `async fn fetch_capabilities` in src/mcp/introspect.rs
- [x] `fn parse_tools_result` / `parse_resources_result` / `parse_prompts_result`
- [x] `send_notification(stdin, &notification)` for `notifications/initialized`
- [x] `tokio::join!` for concurrent list requests
- [x] `pub mod introspect` in src/mcp/mod.rs
- [x] No `unwrap()` in src/mcp/introspect.rs
- [x] No panic on server error responses
- [x] `run_introspection` called in src/supervisor.rs on first Healthy
- [x] `HealthStatus::Healthy` match in supervisor introspection trigger
- [x] `introspect_cancel.cancelled()` select arm in supervisor
- [x] `McpCapabilities::default()` reset on ProcessState::Starting
- [x] `"Tools"` column header in src/output.rs
- [x] `introspected_at` used in output.rs for column formatting
- [x] `T/.*R/.*P` format pattern in output.rs
- [x] `fn introspection_captures_correct_counts` in tests/introspection.rs
- [x] `fn introspection_skips_unsupported_capability` in tests/introspection.rs
- [x] `fn introspection_handles_error_response` in tests/introspection.rs
- [x] `fn introspection_timeout` in tests/introspection.rs
- [x] All existing Phase 2 tests still pass
- [x] No `unwrap()` in production code (src/)
- [x] cargo clippy -D warnings passes
