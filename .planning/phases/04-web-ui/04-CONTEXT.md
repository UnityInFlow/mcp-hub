# Phase 4: Web UI - Context

**Gathered:** 2026-04-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Serve an Axum-backed web interface with server status dashboard, tools/resources/prompts browser, SSE log streaming, and a JSON health endpoint for external monitoring. No JavaScript framework — server-rendered HTML with HTMX for interactivity. Single-binary delivery (all assets embedded).

Requirements: WEB-01, WEB-02, WEB-03, WEB-04, WEB-05.

</domain>

<decisions>
## Implementation Decisions

### Visual style & theming
- **D-01:** Clean modern aesthetic — system font, light background, subtle cards/borders, accent colors for status. Not terminal-like except for log viewer.
- **D-02:** Light theme only (no dark mode in v1). Simpler CSS, faster to ship.
- **D-03:** HTMX for client-side interactivity — partial page updates, SSE integration, no custom JavaScript framework. Feels SPA-like but fully server-rendered.
- **D-04:** HTMX library embedded in binary (include_str! or rust-embed). Works offline, zero external runtime dependencies. ~14KB gzipped.

### Dashboard layout (Status page)
- **D-05:** Card grid layout — one card per server in a responsive CSS grid. Each card shows: server name, colored status dot, health state, PID, uptime, restart count, tool count.
- **D-06:** Cards auto-refresh via HTMX polling (hx-trigger="every 3s" on card container). Status updates appear without manual page refresh.
- **D-07:** Top navigation with tab bar: Status | Tools | Logs. HTMX loads tab content for single-page feel. Hub name + summary stats in header bar.

### Tools browser
- **D-08:** Per-server accordion layout. Each server is a collapsible section showing tool/resource/prompt counts in the header. Expand to see details. HTMX lazy-loads details on expand.
- **D-09:** Tool detail level: name + description. Input schema NOT shown inline in v1 (keep it clean). Schema could be added as click-to-expand later.

### Log viewer
- **D-10:** SSE + HTMX swap for live log streaming (hx-ext="sse" with sse-swap). Auto-appends log lines to container.
- **D-11:** Server filtering via clickable pills/chips at top of log viewer. Each pill colored to match server's log prefix. Click toggles server on/off. All active by default. HTMX reconnects SSE with filter param on toggle.
- **D-12:** Terminal-like visual style for log viewer panel — dark background, monospace font, colored server name prefixes. Consistent with docker-compose style established in Phase 2 (D-07). Contrasts intentionally with the clean modern UI for the rest of the page.

### Health endpoint
- **D-13:** GET /health returns JSON with overall hub status and per-server health. Must respond in under 100ms even with 10+ servers (per WEB-05 success criteria).

### Claude's Discretion
- Template engine choice (askama compile-time vs tera runtime vs manual string building)
- CSS approach (embedded stylesheet vs utility classes vs minimal custom)
- Exact auto-refresh polling interval (2-5s range, 3s suggested)
- Card click behavior (navigate to server detail page, or expand in-place)
- SSE event format and reconnection handling
- Static asset embedding approach (include_str! vs rust-embed crate)
- Empty state design (when no servers configured, or all stopped)
- How web server integrates with daemon mode (same process, same port config)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` -- Full feature spec, web UI section, key features checklist
- `.planning/PROJECT.md` -- Project vision, constraints (Axum + server-rendered HTML, no JS framework)

### Research findings
- `.planning/research/STACK.md` -- Recommended crate versions (axum 0.8.x, askama 0.12.x, tower-http 0.6.x)
- `.planning/research/ARCHITECTURE.md` -- Component breakdown, web UI integration points

### Prior phase context
- `.planning/phases/01-config-process-supervisor/01-CONTEXT.md` -- D-16/D-17 (color and verbosity patterns)
- `.planning/phases/02-health-monitoring-logging/02-CONTEXT.md` -- D-05 (ring buffer 10K lines), D-07 (docker-compose log style), D-09 (status table columns)
- `.planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md` -- D-03 (introspection stored in ServerSnapshot), D-05 (Unix socket IPC), D-14 (CLI socket commands)

### Existing code (key files for Phase 4)
- `src/types.rs` -- ServerSnapshot, HealthStatus, McpCapabilities, ProcessState
- `src/logs.rs` -- Ring buffer with per-server log storage (source for SSE streaming)
- `src/daemon.rs` -- Daemon mode, state access patterns
- `src/control.rs` -- IPC control, state queries
- `src/output.rs` -- Color assignment, table formatting patterns

### Ecosystem constraints
- `CLAUDE.md` -- Rust coding standards, no unwrap(), cargo clippy/fmt, server-rendered HTML constraint

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `ServerSnapshot` (types.rs) -- Contains all data needed for status cards: health, capabilities, process state, PID, uptime, restarts
- `McpCapabilities` (types.rs) -- tools/resources/prompts lists for the tools browser page
- `HealthStatus` enum (types.rs) -- Healthy/Degraded/Failed/Unknown for status dots and health endpoint
- Ring buffer in `logs.rs` -- Per-server log storage, source for SSE streaming and log history
- Color assignment in `output.rs` -- Server color palette, reuse for web UI server pills/prefixes

### Established Patterns
- Per-server tokio tasks with CancellationToken -- extend for SSE broadcast tasks
- watch channels for state broadcasting -- web handlers can subscribe to state changes
- Unix socket IPC (daemon.rs) -- web UI server can share state access approach
- serde_json serialization -- reuse for /health JSON endpoint and HTMX JSON responses

### Integration Points
- Web server runs inside the same process as the daemon/supervisor (shares state via Arc)
- SSE log streaming taps into the same ring buffer that CLI `logs --follow` uses
- Health endpoint reads from the same ServerSnapshot watch channels
- Tab bar navigation uses HTMX to swap content without full page reload
- Web UI port configurable in mcp-hub.toml [hub] section (default 3456)

</code_context>

<specifics>
## Specific Ideas

- Log viewer should feel like a terminal embedded in a clean web page -- dark panel with monospace text inside a light-themed app
- Server filter pills with color coding matching the docker-compose-style log prefixes from Phase 2
- HTMX accordion for tools browser keeps initial page load fast (lazy-load details on expand)
- Card grid with auto-refresh gives a "live dashboard" feel without any custom JavaScript
- Health endpoint is a separate concern from the UI -- should work even if no browser is open

</specifics>

<deferred>
## Deferred Ideas

None -- discussion stayed within phase scope

</deferred>

---

*Phase: 04-web-ui*
*Context gathered: 2026-04-06*
