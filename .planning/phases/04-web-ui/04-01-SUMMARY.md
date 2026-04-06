---
phase: 04-web-ui
plan: 01
subsystem: ui
tags: [axum, askama, askama_web, htmx, tower-http, tokio-stream, futures-util, sse]

# Dependency graph
requires:
  - phase: 03-mcp-introspection-daemon-mode
    provides: DaemonState, LogAggregator, ServerHandle, LogLine — web module reads these

provides:
  - src/web/ module with WebState, build_router, start_web_server
  - Embedded static assets (htmx.min.js, htmx-sse.js, style.css)
  - Askama template stubs (base.html, status.html, tools.html, logs.html)
  - HubGlobalConfig with web_port (default 3456) in [hub] TOML section
  - Compilation foundation for all web UI work in Plan 02–04

affects:
  - 04-02 (routes implementation reads WebState and route stubs)
  - 04-03 (SSE implementation reads sse.rs stub and LogAggregator)
  - 04-04 (config generators use HubGlobalConfig.web_port)

# Tech tracking
tech-stack:
  added:
    - axum 0.8 (HTTP server for web UI)
    - askama 0.15 (compile-time Jinja2-like templates)
    - askama_web 0.15 with axum-0.8 feature (IntoResponse integration)
    - tower-http 0.6 with timeout and cors features
    - tokio-stream 0.1 (async streams for SSE in Plan 02)
    - futures-util 0.3 (stream combinators)
    - portpicker 0.1 (dev-dependency for integration tests)
  patterns:
    - "WebState pattern: Arc<WebState> holds handles and log_agg for handler access"
    - "Embedded assets pattern: include_str! macros compile static files into binary"
    - "askama_web derive pattern: #[derive(Template, WebTemplate)] on template structs"
    - "Stub handlers: return empty string, Plan 02 fills in actual data population"

key-files:
  created:
    - src/web/mod.rs (WebState struct, build_router, start_web_server)
    - src/web/assets.rs (embedded HTMX + CSS via include_str!)
    - src/web/routes.rs (StatusPage, ToolsPage, LogsPage template structs + stub handlers)
    - src/web/sse.rs (LogParams, log_stream_handler stub)
    - static/htmx.min.js (HTMX v2.0.4, 50KB, downloaded from unpkg)
    - static/htmx-sse.js (HTMX SSE extension v2.2.2, 9KB, downloaded from unpkg)
    - static/style.css (UI stylesheet with card grid, tabs, log viewer, filter pills)
    - templates/base.html (base layout with header, tab nav, htmx script tags)
    - templates/status.html (server card grid stub with HTMX polling)
    - templates/tools.html (tools accordion stub)
    - templates/logs.html (log viewer stub with SSE connection)
  modified:
    - Cargo.toml (added axum, askama, askama_web, tower-http, tokio-stream, futures-util, portpicker)
    - src/config.rs (added HubGlobalConfig, default_web_port, hub field to HubConfig, tests module)
    - src/logs.rs (format_system_time: private -> pub(crate))
    - src/main.rs (added mod web;)

key-decisions:
  - "askama_web 0.15 with axum-0.8 feature used (not deprecated askama_axum)"
  - "All web module items annotated #![allow(dead_code)] — used only from router, wired in Plan 02"
  - "Static assets embedded via include_str! at compile time (zero runtime dependency)"
  - "Template stubs contain valid Jinja2 syntax but empty data — Plan 02 wires actual data"

patterns-established:
  - "Router pattern: build_router(Arc<WebState>) returns Router for use in start_web_server"
  - "Path params use {param} syntax (axum 0.8), not :param (axum 0.7)"
  - "Tool detail partial: Path(server_name): Path<String> as first param before State"

requirements-completed: [WEB-01]

# Metrics
duration: 30min
completed: 2026-04-06
---

# Phase 4 Plan 01: Web UI Foundation Summary

**Axum 0.8 + askama_web compilation scaffold with embedded HTMX assets, full CSS stylesheet, and template stubs — all 15 files compiled and passing clippy + fmt**

## Performance

- **Duration:** ~30 min
- **Started:** 2026-04-06T19:15:00Z
- **Completed:** 2026-04-06T19:47:33Z
- **Tasks:** 2
- **Files modified:** 15

## Accomplishments

- Added all web UI Cargo dependencies (axum, askama, askama_web, tower-http, tokio-stream, futures-util) — `cargo build` succeeds with 0 errors
- Extended `HubConfig` with `[hub]` section and `web_port` (default 3456) with 2 unit tests confirming TOML parsing
- Created complete `src/web/` module: WebState, build_router, start_web_server, embedded assets, stub route handlers, stub SSE handler
- Downloaded HTMX v2.0.4 (50KB) and HTMX SSE v2.2.2 (9KB) from unpkg, embedded via `include_str!`
- Created full CSS stylesheet and 4 askama template stubs — all compile and pass askama derive macros

## Task Commits

Each task was committed atomically:

1. **Task 1: Add web dependencies and extend HubConfig with [hub] section** - `4c954ef` (feat)
2. **Task 2: Create web module skeleton, static assets, and askama templates** - `929f719` (feat)

**Plan metadata:** (docs commit — see below)

## Files Created/Modified

- `Cargo.toml` — added 6 production + 1 dev dependency for web UI stack
- `src/config.rs` — HubGlobalConfig struct, default_web_port(), hub field, 2 tests
- `src/logs.rs` — format_system_time promoted to pub(crate)
- `src/main.rs` — added `mod web;` declaration
- `src/web/mod.rs` — WebState, build_router, start_web_server
- `src/web/assets.rs` — HTMX_JS, HTMX_SSE_JS, STYLE_CSS constants + serve_* handlers
- `src/web/routes.rs` — ServerCardData, HealthResponse, StatusPage, ToolsPage, LogsPage + stubs
- `src/web/sse.rs` — LogParams, log_stream_handler stub
- `static/htmx.min.js` — HTMX v2.0.4 (downloaded)
- `static/htmx-sse.js` — HTMX SSE v2.2.2 (downloaded)
- `static/style.css` — full UI stylesheet (card grid, tabs, log viewer, filter pills)
- `templates/base.html` — base layout with nav tabs
- `templates/status.html` — server card grid stub
- `templates/tools.html` — tools accordion stub
- `templates/logs.html` — log viewer with SSE connect stub

## Decisions Made

- Used `askama_web` 0.15 with `axum-0.8` feature — the old `askama_axum` crate is deprecated and removed; `askama_web` is the correct replacement
- Added `#![allow(dead_code)]` at file level for `src/web/mod.rs` and `src/web/assets.rs` — all functions are referenced from `build_router` but `build_router` itself isn't called anywhere yet (wired in Plan 02); this avoids `-D warnings` failures without suppressing actual bugs
- Template stubs intentionally contain valid Jinja2 syntax with empty data vectors — askama validate template syntax at compile time, stubs must compile even though data population comes in Plan 02

## Deviations from Plan

None — plan executed exactly as written.

## Issues Encountered

- `cargo fmt --check` failed after initial write due to import ordering (header before IntoResponse) and line length in assets.rs — fixed by running `cargo fmt` before final clippy check. Not a logic issue, formatting only.

## User Setup Required

None — no external service configuration required.

## Known Stubs

The following stubs exist intentionally — Plan 02 will wire actual data:

- `src/web/routes.rs` — `status_page`, `tools_page`, `logs_page`, `status_partial`, `tool_detail_partial` return empty/placeholder responses (servers vec is always empty)
- `src/web/sse.rs` — `log_stream_handler` returns `""` instead of an SSE stream
- `templates/status.html` — card grid loop body is empty (renders no cards yet)
- `templates/tools.html` — tools list body is empty (renders no tools yet)

These stubs are intentional scaffolding for the compilation foundation. Plan 02 implements the full data population and SSE streaming.

## Next Phase Readiness

- Plan 02 can start immediately — all route signatures, template structs, and WebState are in place
- To wire the web server, Plan 02 needs to: call `start_web_server` from daemon mode in main.rs, pass `Arc<WebState>` constructed from `DaemonState`
- SSE streaming requires `tokio-stream` (already in Cargo.toml) and `axum::response::sse::Sse`

---
*Phase: 04-web-ui*
*Completed: 2026-04-06*
