---
phase: 04-web-ui
plan: 02
subsystem: ui
tags: [axum, askama, htmx, sse, routes, templates, health]

# Dependency graph
requires:
  - phase: 04-web-ui
    plan: 01
    provides: WebState, build_router, template stubs, sse.rs stub, routes.rs stub

provides:
  - src/web/routes.rs — full route handler implementations (status_page, status_partial, tools_page, tool_detail_partial, logs_page, health_handler)
  - src/web/sse.rs — SSE log_stream_handler with BroadcastStream and server filtering
  - templates/status.html — card grid with HTMX 3s polling
  - templates/status_partial.html — fragment-only card grid for HTMX swap
  - templates/tools.html — accordion with HTMX lazy-load per server
  - templates/tools_detail.html — tool/resource/prompt detail fragment
  - templates/logs.html — filter pills + SSE-connected log container

affects:
  - 04-03 (config generators read web_port, no routing changes needed)

# Tech tracking
tech-stack:
  added:
    - tokio-stream 'sync' feature (enables BroadcastStream wrapper for broadcast::Receiver)
  patterns:
    - "ServerCardData::from_snapshot pattern: maps ServerSnapshot fields to CSS-class-aware display strings"
    - "Status partial pattern: fragment-only template (no extends) returned by /partials/status for HTMX innerHTML swap"
    - "SSE pattern: BroadcastStream::new(rx).filter_map() with result.ok()? for silent lagged-event drops"
    - "Pre-computed sse_url pattern: handler computes /logs/stream or /logs/stream?server=name, template uses it directly"
    - "ToolDetailPartial pattern: lazy-loaded on accordion expand via hx-trigger='click once'"

key-files:
  created:
    - templates/status_partial.html (fragment-only card grid for HTMX polling)
    - templates/tools_detail.html (tool/resource/prompt accordion detail fragment)
  modified:
    - src/web/routes.rs (full implementations: ServerCardData, StatusPage, StatusPartial, ToolsPage, ToolDetailPartial, LogsPage, HealthResponse + all handlers)
    - src/web/sse.rs (log_stream_handler with BroadcastStream, filter_map, KeepAlive)
    - templates/status.html (card grid with hx-trigger every 3s, state/health CSS classes)
    - templates/tools.html (accordion with HTMX lazy-load)
    - templates/logs.html (filter pills + SSE-connected log-container with pre-computed sse_url)
    - Cargo.toml (tokio-stream: added 'sync' feature)

key-decisions:
  - "tokio-stream 'sync' feature required for BroadcastStream — not enabled by default"
  - "Pre-computed sse_url in LogsPage handler (vs template logic) keeps template clean and avoids Jinja2 string concat"
  - "is_some_and() preferred over map_or(false, ...) per clippy recommendation"
  - "health_handler derives overall status from health string prefix ('failed') rather than enum matching — avoids re-locking state_rx"
  - "SSE data format: HTML div fragment that HTMX swaps directly into log-container (sse-swap='log' + hx-swap='beforeend')"

requirements-completed: [WEB-02, WEB-03, WEB-04, WEB-05]

# Metrics
duration: 15min
completed: 2026-04-06T19:55:46Z
---

# Phase 4 Plan 02: Web UI Routes, SSE Streaming, and Templates Summary

**Full route handler implementations with askama templates — status card grid with HTMX polling, tools accordion with lazy-load, SSE log streaming with server filtering, and /health JSON endpoint**

## Performance

- **Duration:** ~15 min
- **Completed:** 2026-04-06T19:55:46Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments

- Implemented `ServerCardData::from_snapshot` mapping `ServerSnapshot` fields to CSS-class-aware display strings (state_class, health_class, pid, uptime, tool_count)
- Full route handler implementations: status_page, status_partial, tools_page, tool_detail_partial, logs_page, health_handler — all reading from WebState via Arc<Mutex>
- Health endpoint: returns JSON with overall status ("healthy"/"degraded"/"failed") and per-server entries; non-blocking watch channel reads
- SSE log stream: `BroadcastStream` over `broadcast::Receiver<LogLine>` with `filter_map` for server filtering and lagged-error drops
- LogsPage pre-computes SSE URL in the handler ("/logs/stream" or "/logs/stream?server=name") so templates stay clean
- Created `templates/status_partial.html` (fragment-only, no `extends`) and `templates/tools_detail.html` for HTMX partial swaps
- Added `tokio-stream` `sync` feature (required for `BroadcastStream` — not enabled by default)

## Task Commits

1. **Task 1: Implement route handlers (status, tools, health) and complete templates** - `0742876` (feat)
2. **Task 2: Implement SSE log streaming and complete log viewer template** - `5b121c9` (feat)

## Files Created/Modified

- `src/web/routes.rs` — ServerCardData, ServerToolsData, ServerFilterPill, ToolDetail, ResourceDetail, PromptDetail, StatusPage, StatusPartial, ToolsPage, ToolDetailPartial, LogsPage, HealthResponse, ServerHealthEntry + all handler functions
- `src/web/sse.rs` — log_stream_handler with BroadcastStream, filter_map, KeepAlive
- `templates/status.html` — card grid with hx-trigger="every 3s", state_class/health_class CSS dots+badges
- `templates/status_partial.html` — fragment-only card grid (no extends base.html)
- `templates/tools.html` — accordion with hx-get="/partials/tools/{name}", hx-trigger="click once"
- `templates/tools_detail.html` — tool/resource/prompt details fragment
- `templates/logs.html` — filter pills (All + per-server) + SSE-connected log-container
- `Cargo.toml` — tokio-stream: added 'sync' feature

## Decisions Made

- `tokio-stream` needed the `sync` feature flag to expose `BroadcastStream` — this is a compile-time gating, not a runtime feature, so it's safe to enable globally
- Pre-computed `sse_url` field on `LogsPage` keeps template logic simple: template uses `{{ sse_url }}` directly in `sse-connect` attribute without string manipulation
- `is_some_and()` used for filter pill `active` field (clippy prefers this over `map_or(false, ...)`)
- SSE data is an HTML fragment (a `<div class="log-line">`) that HTMX swaps directly via `sse-swap="log"` + `hx-swap="beforeend"`, appending to the log container
- KeepAlive at 15 seconds prevents proxy/browser connection timeouts on idle streams

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added tokio-stream 'sync' feature for BroadcastStream**
- **Found during:** Task 2
- **Issue:** `BroadcastStream` is gated behind `#[cfg(feature = "sync")]` in tokio-stream; the feature was not enabled in Cargo.toml
- **Fix:** Added `features = ["sync"]` to the tokio-stream dependency in Cargo.toml
- **Files modified:** Cargo.toml, Cargo.lock
- **Commit:** 5b121c9

**2. [Rule 1 - Bug] Fixed clippy: map_or(false, ...) -> is_some_and()**
- **Found during:** Task 1
- **Issue:** clippy -D warnings failed on `map_or(false, |s| s == &h.name)` in logs_page handler
- **Fix:** Replaced with `is_some_and(|s| s == &h.name)`
- **Files modified:** src/web/routes.rs
- **Commit:** 0742876

**3. [Rule 1 - Bug] Applied cargo fmt to pre-existing formatting issue in tests/config_reload.rs**
- **Found during:** Task 1 verification
- **Issue:** tests/config_reload.rs had pre-existing import ordering and line length issues that failed `cargo fmt --check`
- **Fix:** Ran `cargo fmt` to apply canonical formatting
- **Files modified:** tests/config_reload.rs
- **Commit:** 0742876

## Known Stubs

None — all route handlers return real data from WebState. The web server is not yet called from `main.rs` with a real `WebState` (that wiring happens when the daemon mode is started), but all handlers are fully implemented.

## Self-Check: PASSED
