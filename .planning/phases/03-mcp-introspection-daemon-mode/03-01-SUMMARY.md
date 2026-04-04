---
plan_id: "03-01"
title: "MCP Protocol Types + Dispatcher Refactor"
status: completed
date_completed: 2026-04-02
---

# Plan 03-01 Execution Summary

## What Was Done

All 6 tasks executed and committed. The dispatcher pattern replaces the per-ping
read-in-place approach from Phase 2. A shared `reader_task` now owns stdout for
the process lifetime; health checks and future introspection tasks share stdin
via `SharedStdin` + `PendingMap` + `IdAllocator`.

## Tasks Completed

### Task 1 — Add MCP introspection types to protocol.rs
- Added `JsonRpcRequest<T>` generic request struct
- Added `JsonRpcNotification` with `initialized()` constructor
- Added `InitializeParams`, `ClientInfo` for the initialize handshake
- Added response types: `InitializeResult`, `ServerCapabilities`, `ServerInfo`,
  `ToolsListResult`, `McpTool`, `ResourcesListResult`, `McpResource`,
  `PromptsListResult`, `McpPrompt`, `PromptArgument`
- Added constructor helpers: `initialize_request`, `tools_list_request`,
  `resources_list_request`, `prompts_list_request`
- Removed `#![allow(dead_code)]` (replaced with targeted forward-looking suppression)

### Task 2 — Add McpCapabilities and extend ServerSnapshot
- Added `McpCapabilities` struct with `tools`, `resources`, `prompts`, `introspected_at`
- Added `capabilities: McpCapabilities` field to `ServerSnapshot`
- Updated `Default` impl for `ServerSnapshot` to include `capabilities`
- Removed `#![allow(dead_code)]` from types.rs (added targeted suppression)

### Task 3 — Create dispatcher module
- Created `src/mcp/dispatcher.rs` with:
  - `PendingMap` and `SharedStdin` type aliases
  - `IdAllocator` with `AtomicU64` (per-server, not global)
  - `reader_task` — owns stdout, routes responses by ID, drains on close
  - `send_request` — registers oneshot before writing, timeout cleans up pending entry
  - `send_notification` — fire-and-forget write
- Added `pub mod dispatcher` to `src/mcp/mod.rs`

### Task 4 — Refactor health.rs to use the dispatcher
- Rewrote `ping_server` to accept `&SharedStdin`, `&PendingMap`, `id`
- Rewrote `run_health_check_loop` to accept `SharedStdin`, `PendingMap`, `Arc<IdAllocator>`
- Removed `BufReader<ChildStdout>` / `ChildStdin` ownership from health.rs
- ID allocation now uses `id_alloc.next_id()` per tick

### Task 5 — Wire dispatcher into supervisor.rs
- Added `Mutex`, `SharedStdin`, `PendingMap`, `IdAllocator` imports
- Replaced old health task spawning with dispatcher pattern:
  - Creates `SharedStdin`, `PendingMap`, `IdAllocator`
  - Spawns `reader_task` owning stdout
  - Spawns health check loop with shared resources
- No `ChildStdin`/`ChildStdout` passed directly to health.rs anymore
- cargo clippy and cargo fmt both clean

### Task 6 — Update tests for dispatcher refactor
- Updated `tests/health_monitor.rs`: uses `SharedStdin + PendingMap + reader_task`
  in all integration tests; added `spawn_responder_with_dispatcher` helper
- Added `tests/dispatcher.rs`:
  - `dispatcher_send_request_returns_correct_response`
  - `dispatcher_send_request_times_out_when_no_response`
  - `reader_task_drains_pending_map_on_stdout_close`
  - `dispatcher_concurrent_requests` (5 concurrent IDs)
  - `id_allocator_starts_at_one`, `id_allocator_increments_monotonically`, `id_allocator_default_matches_new`
- Added `tests/protocol.rs`:
  - `initialize_request_serialization`, `tools_list_request_serialization`,
    `resources_list_request_serialization`, `prompts_list_request_serialization`
  - `notifications_initialized_serialization` (no `id` field)
  - `initialize_result_deserialization`, `initialize_result_minimal_deserialization`
  - `tools_list_result_deserialization`, `mcp_tool_roundtrip`
  - `resources_list_result_deserialization`, `mcp_resource_roundtrip`
  - `prompts_list_result_deserialization`, `mcp_prompt_roundtrip`
- Fixed `ServerSnapshot` construction in `tests/status_table.rs` and
  `tests/integration_phase2.rs` to include `capabilities` via `..ServerSnapshot::default()`

## Final State

| Check | Result |
|---|---|
| `cargo build` | clean (0 errors) |
| `cargo clippy -- -D warnings` | clean (0 issues) |
| `cargo fmt -- --check` | clean |
| `cargo test` | 106 passed, 0 failed |
| No `unwrap()` in src/ | confirmed |
| No `ChildStdout` passed to health.rs | confirmed |
| `reader_task` drains pending map on close | confirmed by test |

## Key Architectural Decision

The `reader_task` + `PendingMap` + `oneshot` pattern was implemented exactly as
specified. The `IdAllocator` is per-server (not global) so IDs are unique within
a server's session even across health pings and future introspection requests.
The `send_request` function registers the waiter before writing the request to
eliminate any race where a fast server responds before the map entry is inserted.
