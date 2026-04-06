---
phase: 04-web-ui
plan: 03
subsystem: ui
tags: [axum, web-server, main, foreground, daemon, tests, sse, routes]

# Dependency graph
requires:
  - phase: 04-web-ui
    plan: 02
    provides: WebState, build_router, route handlers, SSE handler, templates

provides:
  - src/main.rs — web server wired into both daemon and foreground modes
  - src/web/routes.rs — in-module unit tests for all WEB requirements
  - test-web.toml — test config for human verification step

affects:
  - Phase 5 (if any) — web server now always starts alongside managed servers

# Tech tracking
tech-stack:
  added:
    - tower 0.5 (dev-dep) — ServiceExt::oneshot for route testing
    - http-body-util 0.1 (dev-dep) — axum::body::to_bytes in tests
  patterns:
    - "Arc<Mutex<Vec<ServerHandle>>> pattern for foreground mode: handles wrapped before web server spawn so both share ownership"
    - "run_foreground_loop_shared pattern: locks Arc<Mutex> briefly per command, drops lock before awaiting signal"
    - "In-module test pattern: #[cfg(test)] mod tests inside src/web/routes.rs; uses build_router + ServiceExt::oneshot"

key-files:
  created:
    - templates/status_partial.html (applied from Plan 02 -- missed in merge)
    - templates/tools_detail.html (applied from Plan 02 -- missed in merge)
    - test-web.toml (test config for human browser verification)
  modified:
    - src/main.rs (web server wired into daemon and foreground modes; run_foreground_loop_shared)
    - src/web/routes.rs (full Plan 02 implementation applied; 8 in-module unit tests added)
    - src/web/sse.rs (full Plan 02 implementation applied -- missed in merge)
    - templates/status.html (server card body added -- Plan 02 content missed in merge)
    - templates/tools.html (accordion body added -- Plan 02 content missed in merge)
    - templates/logs.html (filter pills and sse_url applied -- Plan 02 content)
    - Cargo.toml (tokio-stream sync feature restored; tower + http-body-util dev-deps added)
    - tests/config_reload.rs (HubConfig initializer: added hub: Default::default())
    - tests/integration_phase2.rs (HubConfig initializer: added hub: Default::default())

key-decisions:
  - "Plan 02 source code was not merged into main branch (merge commit 69142b6 only included STATE.md, config.json, tests/config_reload.rs). Applied all missing Plan 02 source changes as a Rule 3 deviation before proceeding."
  - "run_foreground_loop replaced by run_foreground_loop_shared: takes Arc<Mutex<Vec<ServerHandle>>> instead of &[ServerHandle] so handles can be shared with the web server task"
  - "web_task.abort() rather than .await in daemon shutdown: CancellationToken already signals graceful shutdown; abort() is a safety net"
  - "In-module tests (mod tests inside routes.rs) rather than tests/ directory: binary crate tests/ cannot import private modules; in-module tests access build_router directly"

requirements-completed: [WEB-01, WEB-02, WEB-03, WEB-04, WEB-05]

# Metrics
duration: 25min
completed: 2026-04-06T22:30:00Z
---

# Phase 4 Plan 03: Wire Web Server and Unit Tests Summary

**Web server wired into both daemon and foreground modes; 8 in-module route unit tests covering all WEB requirements; awaiting human browser verification**

## Performance

- **Duration:** ~25 min
- **Completed:** 2026-04-06T22:30:00Z
- **Tasks:** 1/2 complete (Task 2 is human checkpoint)
- **Files modified:** 9

## Accomplishments

- Web server spawned in daemon mode after `DaemonState` creation; `web_task.abort()` in shutdown sequence
- Web server spawned in foreground mode with `Arc<Mutex<Vec<ServerHandle>>>` wrapping before the web spawn; `run_foreground_loop` replaced by `run_foreground_loop_shared`
- 8 in-module unit tests in `src/web/routes.rs` covering: status_page_returns_200, status_partial_returns_fragment, tools_page_returns_200, log_stream_returns_sse_content_type, health_returns_json_with_status_and_servers, health_responds_under_100ms, static_htmx_returns_javascript, static_css_returns_stylesheet
- All tests pass; clippy -D warnings passes; cargo fmt --check passes
- `test-web.toml` created for human browser verification step

## Task Commits

1. **Task 1: Wire web server into main.rs and write unit tests** - `b92f0fb` (feat)

## Files Created/Modified

- `src/main.rs` — web server wired into daemon and foreground modes; `run_foreground_loop_shared` replaces `run_foreground_loop`
- `src/web/routes.rs` — full Plan 02 handler implementations + 8 in-module tests
- `src/web/sse.rs` — full Plan 02 SSE implementation applied
- `templates/status.html` — server card body cards restored from Plan 02
- `templates/status_partial.html` — new file: fragment-only card grid
- `templates/tools.html` — accordion body with HTMX lazy-load restored from Plan 02
- `templates/tools_detail.html` — new file: tool/resource/prompt detail fragment
- `templates/logs.html` — filter pills + pre-computed sse_url applied from Plan 02
- `Cargo.toml` — tokio-stream sync feature; tower + http-body-util dev-deps
- `tests/config_reload.rs` — HubConfig hub field fix
- `tests/integration_phase2.rs` — HubConfig hub field fix
- `test-web.toml` — test config for human verification

## Decisions Made

- Plan 02 source code (routes.rs, sse.rs, templates) was applied as a Rule 3 deviation — the worktree's merge commit did not include the actual source file changes from the Plan 02 branch
- `run_foreground_loop_shared` acquires the mutex lock briefly for each stdin command and drops it before awaiting the next signal — prevents holding the lock during slow stdin reads
- In-module tests are the correct pattern for binary crates: `tests/` directory files cannot access internal modules directly

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Applied missing Plan 02 source code to worktree**
- **Found during:** Pre-task analysis
- **Issue:** The merge commit 69142b6 (Merge worktree-agent-a9f36a93 Plan 04-02) only merged `.planning/STATE.md`, `.planning/config.json`, and `tests/config_reload.rs`. The actual source changes from commits 0742876 and 5b121c9 (routes.rs, sse.rs, all templates) were not included in the merge. The worktree was showing Plan 01 stub versions.
- **Fix:** Applied all Plan 02 source changes from the Plan 02 branch commits into the main worktree
- **Files modified:** src/web/routes.rs, src/web/sse.rs, templates/status.html, templates/status_partial.html, templates/tools.html, templates/tools_detail.html, templates/logs.html, Cargo.toml (tokio-stream sync feature)
- **Commit:** b92f0fb

**2. [Rule 1 - Bug] Fixed missing `hub` field in HubConfig initializers in test files**
- **Found during:** Task 1 verification (cargo test run)
- **Issue:** `tests/config_reload.rs` and `tests/integration_phase2.rs` constructed `HubConfig { servers }` without the `hub` field added in Plan 01
- **Fix:** Added `hub: Default::default()` to all `HubConfig` struct initializations in both test files
- **Files modified:** tests/config_reload.rs, tests/integration_phase2.rs
- **Commit:** b92f0fb

## Known Stubs

None — all route handlers return real data from WebState. Web server wired into both daemon and foreground modes.

## Awaiting Human Verification (Task 2)

Task 2 is a `checkpoint:human-verify`. To verify:

```bash
cargo run -- start --config test-web.toml
```

Then open http://127.0.0.1:3456 and verify all 9 steps in the plan's how-to-verify section.

## Self-Check: PASSED

- `b92f0fb` exists in git log
- `src/main.rs` contains `web::start_web_server` (2 occurrences)
- `src/web/routes.rs` contains `mod tests` with 8 test functions
- All 8 route tests pass: `cargo test --bin mcp-hub -- web::routes::tests`
- `cargo clippy -- -D warnings` passes
- `cargo fmt --check` passes
