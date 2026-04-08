---
phase: 05-config-generation
plan: "02"
subsystem: gen-config
tags: [config-generation, live-mode, ipc, rust, tool-names]
dependency_graph:
  requires:
    - src/control.rs (DaemonRequest::Status, DaemonResponse, send_daemon_command)
    - src/gen_config.rs (ServerLiveInfo, render_claude_config, render_cursor_config — from Plan 01)
    - src/daemon.rs (socket_path — MCP_HUB_SOCKET env override)
  provides:
    - src/control.rs (extended Status response with tool_names, resource_names, prompt_names arrays)
    - src/gen_config.rs (parse_live_info — fully wired, extracts tool_names from daemon response)
    - tests/gen_config_test.rs (--live error path integration tests)
  affects:
    - mcp-hub status output (now includes tool_names/resource_names/prompt_names — backward-compatible)
tech_stack:
  added: []
  patterns:
    - anyhow::ensure! for response.ok check before field extraction
    - Safe JSON accessor chain: as_array().map(...).unwrap_or_default() — no panics on malformed input (T-05-07)
    - MCP_HUB_SOCKET env override pattern for daemon-isolated integration tests
key_files:
  created: []
  modified:
    - src/control.rs (Status response extended with tool_names/resource_names/prompt_names)
    - src/gen_config.rs (parse_live_info rewritten, 6 new unit tests, stale doc comments cleaned)
    - tests/gen_config_test.rs (2 new --live integration tests)
decisions:
  - parse_live_info checks response.ok first via anyhow::ensure! before any field access — consistent with T-05-07 mitigation
  - tool_names array added alongside existing tools count field — backward-compatible extension, mcp-hub status callers unaffected
  - Integration tests for --live use MCP_HUB_SOCKET env override rather than mocking — tests the real binary exit path
metrics:
  duration_secs: 480
  completed_date: "2026-04-08"
  tasks_completed: 2
  tasks_total: 2
  files_created: 0
  files_modified: 3
---

# Phase 05 Plan 02: --live Mode IPC Extension and parse_live_info Summary

Extended the IPC Status response with tool name arrays and rewired `parse_live_info` to extract them, completing the `--live` flag for `gen-config` so tool names appear as `// server: tools=[a, b, c]` comments in generated config output.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Extend IPC Status + implement parse_live_info | 24f4036 | src/control.rs, src/gen_config.rs |
| 2 | CLI integration tests for --live error path | 6cf7e0b | tests/gen_config_test.rs |
| cleanup | Remove stale Plan 02 stub doc comments | 408256e | src/gen_config.rs |

## What Was Built

**src/control.rs** — `DaemonRequest::Status` response extended with three new fields (backward-compatible, existing count fields remain):
- `"tool_names"`: `Vec<&str>` — names of all introspected MCP tools
- `"resource_names"`: `Vec<&str>` — names of all introspected MCP resources
- `"prompt_names"`: `Vec<&str>` — names of all introspected MCP prompts

**src/gen_config.rs** — `parse_live_info` rewritten from the Plan 01 stub:
- Checks `response.ok` via `anyhow::ensure!` before any field access
- Extracts `tool_names` from `s["tool_names"].as_array()` with safe fallback to empty Vec
- Extracts `resource_count` and `prompt_count` from counts
- 6 new unit tests covering: success with tools, failure response, empty array, running-with-tools comment, stopped-server WARNING, stopped-with-cached-tools (both WARNING and tools comment)

**tests/gen_config_test.rs** — 2 new integration tests:
- `live_flag_without_daemon_errors`: `--format claude --live` exits non-zero with "daemon"/"running" in stderr
- `live_flag_cursor_without_daemon_errors`: same for `--format cursor`
- Both use `MCP_HUB_SOCKET=/tmp/mcp-hub-test-nonexistent.sock` env override

## Test Results

- 14 unit tests in `src/gen_config.rs` — all pass (8 from Plan 01 + 6 new)
- 10 integration tests in `tests/gen_config_test.rs` — all pass (8 from Plan 01 + 2 new)
- Total suite: 182 tests pass across 19 suites
- `cargo clippy -- -D warnings` — no issues
- `cargo fmt --check` — clean

## Verification

```
cargo build                          -- 0 errors, 0 warnings
cargo clippy -- -D warnings          -- no issues
cargo fmt --check                    -- clean
cargo test --lib gen_config          -- 14/14 pass
cargo test --test gen_config_test    -- 10/10 pass
cargo test                           -- 182/182 pass
```

Manual: `mcp-hub gen-config --format claude --live` without daemon exits non-zero with error message containing "daemon" and "Is the daemon running?" — verified via integration test with socket env override.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Cleanup] Removed stale "extended in Plan 02" doc comments**
- **Found during:** Stub scan before SUMMARY creation
- **Issue:** `ServerLiveInfo.resource_count` and `prompt_count` doc comments still said "(extended in Plan 02)" — Plan 02 IS this plan; the fields are now wired.
- **Fix:** Updated doc comments to remove stale forward-reference.
- **Files modified:** src/gen_config.rs
- **Commit:** 408256e

### TDD Note

The integration test RED phase for Task 2 showed tests passing immediately (not failing first). This is because the `--live` error path was already fully wired in Plan 01's `main.rs` dispatch arm — `send_daemon_command` propagates a connection error containing "daemon" to stderr. The tests document and lock the already-correct contract. This is acceptable: the TDD RED requirement applies when the implementation is absent; here the implementation was present but untested.

## Known Stubs

None. All Plan 01 stubs resolved:
- `ServerLiveInfo.tool_names` now populated from `s["tool_names"]` in daemon response
- `ServerLiveInfo.resource_count` now populated from `s["resources"]` count
- `ServerLiveInfo.prompt_count` now populated from `s["prompts"]` count

## Threat Flags

No new network endpoints, auth paths, or trust boundary surface introduced. The IPC response extension adds array fields to the existing Unix socket response — same trust boundary as before (T-05-05, T-05-06, T-05-07 all remain as accepted/mitigated in the plan's threat model). The `parse_live_info` safe accessor pattern directly implements the T-05-07 mitigation.

## Self-Check: PASSED

- FOUND commit: 24f4036 (feat(05-02): extend IPC Status with tool_names and wire parse_live_info)
- FOUND commit: 6cf7e0b (test(05-02): integration tests for --live error path without daemon)
- FOUND commit: 408256e (chore(05-02): remove stale Plan 02 stub comments)
- FOUND: src/control.rs contains "tool_names"
- FOUND: src/gen_config.rs contains "pub fn parse_live_info"
- FOUND: tests/gen_config_test.rs contains "live_flag_without_daemon_errors"
- VERIFIED: 182 tests pass, clippy clean, fmt clean
