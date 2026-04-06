# Phase 4: Web UI - Research

**Researched:** 2026-04-06
**Domain:** Axum 0.8 web server, SSE log streaming, HTMX, askama templates, embedded assets
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- **D-01:** Clean modern aesthetic — system font, light background, subtle cards/borders, accent colors for status. Not terminal-like except for log viewer.
- **D-02:** Light theme only (no dark mode in v1). Simpler CSS, faster to ship.
- **D-03:** HTMX for client-side interactivity — partial page updates, SSE integration, no custom JavaScript framework. Feels SPA-like but fully server-rendered.
- **D-04:** HTMX library embedded in binary (include_str! or rust-embed). Works offline, zero external runtime dependencies. ~14KB gzipped.
- **D-05:** Card grid layout — one card per server in a responsive CSS grid. Each card shows: server name, colored status dot, health state, PID, uptime, restart count, tool count.
- **D-06:** Cards auto-refresh via HTMX polling (hx-trigger="every 3s" on card container). Status updates appear without manual page refresh.
- **D-07:** Top navigation with tab bar: Status | Tools | Logs. HTMX loads tab content for single-page feel. Hub name + summary stats in header bar.
- **D-08:** Per-server accordion layout. Each server is a collapsible section showing tool/resource/prompt counts in the header. Expand to see details. HTMX lazy-loads details on expand.
- **D-09:** Tool detail level: name + description. Input schema NOT shown inline in v1.
- **D-10:** SSE + HTMX swap for live log streaming (hx-ext="sse" with sse-swap). Auto-appends log lines to container.
- **D-11:** Server filtering via clickable pills/chips at top of log viewer. Each pill colored to match server's log prefix. Click toggles server on/off. All active by default. HTMX reconnects SSE with filter param on toggle.
- **D-12:** Terminal-like visual style for log viewer panel — dark background, monospace font, colored server name prefixes.
- **D-13:** GET /health returns JSON with overall hub status and per-server health. Must respond in under 100ms even with 10+ servers.

### Claude's Discretion
- Template engine choice (askama compile-time vs tera runtime vs manual string building)
- CSS approach (embedded stylesheet vs utility classes vs minimal custom)
- Exact auto-refresh polling interval (2–5s range, 3s suggested)
- Card click behavior (navigate to server detail page, or expand in-place)
- SSE event format and reconnection handling
- Static asset embedding approach (include_str! vs rust-embed crate)
- Empty state design (when no servers configured, or all stopped)
- How web server integrates with daemon mode (same process, same port config)

### Deferred Ideas (OUT OF SCOPE)
None — discussion stayed within phase scope.
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| WEB-01 | Hub serves a web UI on a configurable port (default 3456) | HubConfig needs `[hub]` section; axum TcpListener on configurable addr; port stored in config |
| WEB-02 | Status page shows all servers with name, state, PID, uptime, restart count, health, tool count | ServerSnapshot watch channels already hold all required data; card grid via askama template + HTMX polling |
| WEB-03 | Tools browser page shows tools/resources/prompts per server | McpCapabilities in ServerSnapshot.capabilities; accordion template with HTMX lazy-load on expand |
| WEB-04 | Log viewer streams logs via SSE (all servers or filtered by server) | LogAggregator.subscribe() returns broadcast::Receiver; BroadcastStream + axum Sse response; filter via query param |
| WEB-05 | Health endpoint at /health returns JSON for external monitoring | Pure read from watch channels; no I/O; serde_json serialization |
</phase_requirements>

---

## Summary

Phase 4 adds an Axum HTTP server inside the same daemon process that already manages MCP servers. All data the web UI needs is already live in the codebase: `ServerHandle.state_rx` watch channels hold snapshots with process state, health, PID, uptime, restarts, and MCP capabilities; `LogAggregator.subscribe()` delivers a `broadcast::Receiver<LogLine>` for SSE log streaming.

The main new work is: (1) adding a `[hub]` section to `HubConfig` for the web port, (2) implementing the Axum router with five routes, (3) writing askama templates for the three pages, and (4) embedding HTMX and a CSS stylesheet in the binary. The SSE log stream is the trickiest piece — it requires bridging `tokio::sync::broadcast` to a `futures::Stream`, which the `tokio-stream` crate's `BroadcastStream` wrapper handles cleanly.

The key architectural finding: axum 0.8 ships SSE support in the core crate (no extra features needed beyond `tokio`), integrates cleanly with `CancellationToken` via `.with_graceful_shutdown()`, and the `askama` + `askama_web` pairing gives compile-time HTML templates that implement axum's `IntoResponse` directly. Version divergence is significant: askama jumped to 0.15.6 (from the 0.12 in STACK.md), the `askama_axum` integration crate was removed at 0.13, and the replacement is `askama_web` with the `"axum-0.8"` feature.

**Primary recommendation:** Use `axum 0.8.8`, `askama 0.15.6` + `askama_web 0.15.2` (feature `"axum-0.8"`), `tower-http 0.6.8`, and `tokio-stream 0.1` for BroadcastStream. Embed HTMX and CSS via `include_str!` macros in a `web/assets.rs` module — zero extra compile-time dependency over rust-embed.

---

## Standard Stack

### Core Web

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `axum` | 0.8.8 | HTTP server, routing, SSE response, State extractor | Standard Tokio-first Rust web framework; already decided in STACK.md |
| `askama` | 0.15.6 | Compile-time HTML templates (Jinja2-like syntax) | Compile-time checking, zero runtime overhead; already in STACK.md — but version jumped |
| `askama_web` | 0.15.2 | Implements axum `IntoResponse` for askama templates | The `askama_axum` sub-crate was removed at askama 0.13; `askama_web` is the maintained replacement |
| `tower-http` | 0.6.8 | Timeout layer for request handling | Already in STACK.md; needed for `/health` latency guarantee |
| `tokio-stream` | 0.1 | `BroadcastStream` wrapper to convert broadcast::Receiver into Stream | Required for SSE + broadcast channel bridge |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `futures-util` | 0.3 | `StreamExt::filter` and `StreamExt::map` on BroadcastStream | Filtering SSE stream by server name |

### What NOT to Add

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `askama_web` | Implement `IntoResponse` manually | One-time boilerplate vs. maintained crate; manual is fine for 3 templates |
| `tokio-stream` | `async_stream` macro | `BroadcastStream` is cleaner; `async_stream` needs more plumbing |
| `rust-embed` | `include_str!` macro | `rust-embed` adds a proc-macro dep; `include_str!` is stdlib, sufficient for 2 static files |

**Version verification (crates.io, 2026-04-06):**
- `axum`: 0.8.8 (verified)
- `askama`: 0.15.6 (verified)
- `askama_web`: 0.15.2 (verified)
- `tower-http`: 0.6.8 (verified)
- `tokio-stream`: ships with tokio ecosystem, version 0.1.x (stable)

**Cargo.toml additions:**
```toml
[dependencies]
axum = "0.8"
askama = "0.15"
askama_web = { version = "0.15", features = ["axum-0.8"] }
tower-http = { version = "0.6", features = ["timeout", "cors"] }
tokio-stream = "0.1"
futures-util = "0.3"
```

---

## Architecture Patterns

### Recommended Module Structure

```
src/
├── web/
│   ├── mod.rs          # router() fn, WebState struct, start_web_server()
│   ├── routes.rs       # GET /, GET /tools, GET /health, GET /logs (SSE)
│   ├── sse.rs          # SSE log stream handler and broadcast bridge
│   └── assets.rs       # include_str! for htmx.min.js and style.css
├── templates/           # askama template files (alongside src/ or in root)
│   ├── base.html        # base layout with nav tabs, header bar
│   ├── status.html      # card grid (extends base)
│   ├── tools.html       # accordion per server (extends base)
│   └── logs.html        # log viewer + filter pills (extends base)
```

Askama looks for templates relative to the crate root (configurable). Place them in a `templates/` directory at the crate root.

### Pattern 1: Axum Router with Shared State

The web server runs as a `tokio::spawn`ed task inside the same process as the daemon/supervisor. It shares `Arc<WebState>` which holds `Arc<Vec<ServerHandle>>` (read-only access to watch channels) and `Arc<LogAggregator>`.

```rust
// Source: axum 0.8 official docs — State extractor pattern
use axum::{Router, routing::get, extract::State};
use std::sync::Arc;

pub struct WebState {
    pub handles: Arc<Vec<crate::supervisor::ServerHandle>>,
    pub log_agg: Arc<crate::logs::LogAggregator>,
}

pub fn router(state: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(routes::status_page))
        .route("/tools", get(routes::tools_page))
        .route("/logs", get(sse::log_stream_handler))
        .route("/health", get(routes::health_handler))
        .with_state(state)
}
```

**Critical: `Vec<ServerHandle>` must be wrapped in `Arc` before being passed to the web task.** The handles are created in `main.rs` and cannot be moved into the web task while also being used by the daemon loop. Use `Arc::clone` — the watch channels inside handles are already `Clone`.

### Pattern 2: Graceful Shutdown with CancellationToken

The project already uses `tokio_util::sync::CancellationToken` throughout. Axum 0.8 integrates cleanly:

```rust
// Source: axum 0.8 shutdown discussion #2565
pub async fn start_web_server(
    addr: std::net::SocketAddr,
    state: Arc<WebState>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}
```

Spawn this from `main.rs` daemon start path alongside the control socket task.

### Pattern 3: SSE from Broadcast Channel

The `LogAggregator` already has a `subscribe()` method returning `broadcast::Receiver<LogLine>`. Bridge to SSE:

```rust
// Source: axum SSE docs + tokio-stream BroadcastStream pattern
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use futures_util::Stream;
use std::convert::Infallible;

pub async fn log_stream_handler(
    State(state): State<Arc<WebState>>,
    Query(params): Query<LogParams>, // ?server=name for filtering
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_agg.subscribe();
    let filter_server = params.server.clone();

    let stream = BroadcastStream::new(rx)
        .filter_map(move |result| {
            let filter = filter_server.clone();
            async move {
                match result {
                    Ok(line) => {
                        // Apply server filter if specified
                        if let Some(ref name) = filter {
                            if &line.server != name {
                                return None;
                            }
                        }
                        let data = format!("{} | {} {}", line.server, line.timestamp_str(), line.message);
                        Some(Ok(Event::default().data(data)))
                    }
                    Err(_lagged) => None, // Skip lagged events silently
                }
            }
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

`BroadcastStream` wraps `broadcast::RecvError::Lagged` as a `BroadcastStreamRecvError` — filter it out with `filter_map` rather than surfacing it to the client.

### Pattern 4: Askama Template with axum_web IntoResponse

```rust
// Source: askama_web 0.15 docs, feature "axum-0.8"
use askama::Template;
use askama_web::WebTemplate;

#[derive(Template, WebTemplate)]
#[template(path = "status.html")]
pub struct StatusTemplate {
    pub servers: Vec<ServerCardData>,
    pub total: usize,
    pub healthy: usize,
}

// In route handler:
pub async fn status_page(
    State(state): State<Arc<WebState>>,
) -> StatusTemplate {
    let servers: Vec<ServerCardData> = state.handles.iter()
        .map(|h| ServerCardData::from_snapshot(h.name.clone(), &h.state_rx.borrow()))
        .collect();
    let healthy = servers.iter().filter(|s| s.is_healthy).count();
    StatusTemplate { total: servers.len(), healthy, servers }
}
```

The `#[derive(WebTemplate)]` from `askama_web` implements `axum::response::IntoResponse`, returning `200 OK` with `Content-Type: text/html; charset=utf-8`.

### Pattern 5: HTMX Polling for Status Cards

The status page template HTML pattern:

```html
<!-- status.html fragment -->
<div id="server-cards"
     hx-get="/partials/status"
     hx-trigger="every 3s"
     hx-swap="innerHTML">
  {% for server in servers %}
  <div class="server-card">
    <span class="status-dot status-{{ server.state }}"></span>
    <strong>{{ server.name }}</strong>
    ...
  </div>
  {% endfor %}
</div>
```

This requires a `/partials/status` route returning only the card grid HTML (not the full page). Add `GET /partials/status` to the router alongside the full-page routes.

### Pattern 6: HTMX SSE Log Streaming

```html
<!-- logs.html fragment -->
<div id="log-stream"
     hx-ext="sse"
     sse-connect="/logs?server={{ active_filter }}"
     sse-swap="message"
     hx-swap="beforeend">
</div>
```

When a filter pill is clicked, HTMX makes a request to update `sse-connect` attribute and reconnects. Implement filter pills as `<a>` tags with `hx-get` pointing to `/partials/logs?server=name` which re-renders the filter pills and the SSE container with the new `sse-connect` URL.

### Pattern 7: Embedded Static Assets

```rust
// web/assets.rs
pub const HTMX_JS: &str = include_str!("../../static/htmx.min.js");
pub const STYLE_CSS: &str = include_str!("../../static/style.css");
```

Add routes to serve these:
```rust
.route("/static/htmx.min.js", get(|| async { 
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], HTMX_JS)
}))
.route("/static/style.css", get(|| async {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}))
```

### Pattern 8: /health JSON Endpoint

```rust
// Source: axum docs, serde_json pattern
use axum::Json;

#[derive(serde::Serialize)]
pub struct HealthResponse {
    pub status: &'static str,   // "healthy" | "degraded" | "failed"
    pub servers: Vec<ServerHealth>,
}

#[derive(serde::Serialize)]
pub struct ServerHealth {
    pub name: String,
    pub state: String,
    pub health: String,
}

pub async fn health_handler(
    State(state): State<Arc<WebState>>,
) -> Json<HealthResponse> {
    let servers: Vec<ServerHealth> = state.handles.iter()
        .map(|h| {
            let snap = h.state_rx.borrow();
            ServerHealth {
                name: h.name.clone(),
                state: snap.process_state.to_string(),
                health: snap.health.to_string(),
            }
        })
        .collect();

    let overall = if servers.iter().all(|s| s.health == "healthy") {
        "healthy"
    } else if servers.iter().any(|s| s.health == "failed") {
        "failed"
    } else {
        "degraded"
    };

    Json(HealthResponse { status: overall, servers })
}
```

`watch::Receiver::borrow()` is non-blocking (O(1)) — 10+ servers will complete in microseconds, well under the 100ms requirement.

### Pattern 9: HubConfig `[hub]` Section

`HubConfig` currently has no `[hub]` section. Add:

```rust
// config.rs addition
fn default_web_port() -> u16 { 3456 }

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HubGlobalConfig {
    #[serde(default = "default_web_port")]
    pub web_port: u16,

    /// Bind address for the web UI. Defaults to "127.0.0.1".
    #[serde(default)]
    pub web_host: Option<String>,
}

// In HubConfig:
pub struct HubConfig {
    #[serde(default)]
    pub hub: HubGlobalConfig,
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}
```

TOML config example:
```toml
[hub]
web_port = 3456

[servers.my-server]
command = "npx"
args = ["-y", "@my/mcp-server"]
```

### Anti-Patterns to Avoid

- **Holding watch borrow across await:** `state_rx.borrow()` returns a `Ref<T>` that must not be held across an `.await` point. Clone the snapshot immediately: `let snap = h.state_rx.borrow().clone();`
- **Using `std::sync::RwLock` in async handlers:** Always use `tokio::sync::RwLock` if lock is held across await — but prefer watch channels (already in use) over adding new RwLocks.
- **Treating `BroadcastStreamRecvError::Lagged` as fatal:** Lagged means the subscriber fell behind; silently drop the gap and continue. Never `unwrap()` on `BroadcastStream` items.
- **Full page re-renders for polling:** Return only the card grid fragment at `/partials/status`, not the full HTML page, so HTMX can swap just the inner content.
- **Blocking the Tokio runtime in route handlers:** All handlers must be async. Never call `LogBuffer::snapshot_last` directly from a sync context.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Broadcast → Stream bridge | Manual `async_stream` wrapper | `tokio_stream::wrappers::BroadcastStream` | Handles lagged errors, proper Unpin + Stream impl |
| Template → IntoResponse | Custom wrapper struct | `askama_web` with `"axum-0.8"` feature | Correct Content-Type, error handling, actively maintained |
| SSE framing | Manual `data: ...\n\n` formatting | `axum::response::sse::Event` | Spec-correct SSE format, handles keep-alive |
| Server-Sent Event reconnect | Manual EventSource JS | HTMX `hx-ext="sse"` | HTMX handles reconnect transparently; zero JS to write |
| Static asset cache headers | Manual ETag/Cache-Control | `tower-http` (or inline response with `Cache-Control`) | tower-http middleware handles this correctly |

**Key insight:** The log streaming pipeline (broadcast channel → BroadcastStream → SSE stream → HTTP response) is fully covered by existing crates. The only custom code needed is the `filter_map` that applies server name filtering.

---

## Common Pitfalls

### Pitfall 1: `watch::Ref` Held Across Await
**What goes wrong:** `h.state_rx.borrow()` returns a `Ref<ServerSnapshot>` that holds a lock. If held across `.await`, the lock is never released, causing deadlocks.
**Why it happens:** Rust allows `Ref` to be stored in variables; the compiler does not warn unless the variable crosses an await point.
**How to avoid:** Always immediately clone: `let snap = h.state_rx.borrow().clone();`
**Warning signs:** Tokio deadlock panics at runtime with "task is blocking on a locked watch channel."

### Pitfall 2: `broadcast::RecvError::Lagged` Breaks SSE Stream
**What goes wrong:** If the SSE client is slow, the broadcast sender advances past the receiver's buffer. `BroadcastStream` wraps this as `Err(BroadcastStreamRecvError::Lagged(n))`. If not handled, this terminates the stream.
**Why it happens:** The log aggregator's broadcast channel has capacity 1024. A slow client with many active servers can fall behind.
**How to avoid:** Use `filter_map` on the `BroadcastStream` and return `None` for lagged errors. The HTMX SSE extension reconnects automatically on stream close.
**Warning signs:** Log viewer in browser disconnects shortly after opening.

### Pitfall 3: askama 0.13+ Removed `askama_axum`
**What goes wrong:** Adding `askama_axum` as a dependency fails — the crate was archived. Building `impl IntoResponse` manually for each template also works but is boilerplate.
**Why it happens:** The askama project restructured its integration crates at 0.13. The old `askama_axum` crate is on crates.io but unmaintained.
**How to avoid:** Use `askama_web = { version = "0.15", features = ["axum-0.8"] }` alongside `askama`. Derive both `#[derive(Template)]` and `#[derive(WebTemplate)]` on each template struct.

### Pitfall 4: Axum 0.8 Path Parameter Syntax
**What goes wrong:** Routes defined with `/:name` syntax silently fail or produce confusing errors in axum 0.8.
**Why it happens:** Axum 0.8 changed to `/{name}` syntax (OpenAPI-aligned). Old syntax is not accepted.
**How to avoid:** Use `/{name}` for path params: `"/tools/{server_name}"`.

### Pitfall 5: Web Server Port Already in Use
**What goes wrong:** If the user has another service on port 3456, the `TcpListener::bind` fails with `Address already in use`. The entire hub start fails.
**Why it happens:** Port 3456 is the hardcoded default; not checked before bind.
**How to avoid:** Wrap `TcpListener::bind` in a clear error: `"Failed to bind web UI on port {port}: address already in use. Set [hub] web_port = <other> in mcp-hub.toml"`. Do not panic.

### Pitfall 6: `handles` Vec Ownership Split Between Web and Daemon
**What goes wrong:** After `start_all_servers` returns a `Vec<ServerHandle>`, the daemon loop owns `handles` and also passes them to the control socket via `DaemonState`. Adding the web server as a third consumer requires re-thinking ownership.
**Why it happens:** `Vec<ServerHandle>` is not `Clone`. The existing `DaemonState` already wraps them in `Arc<Mutex<Vec<ServerHandle>>>`.
**How to avoid:** The web server should NOT need write access to handles. Expose a separate `Arc<Vec<ServerHandle>>` snapshot (or `Arc<Mutex<Vec<ServerHandle>>>` clone from `DaemonState`) for read-only watch channel access. The `WebState` only needs `handles` for `.state_rx.borrow()` — no mutation required.

### Pitfall 7: HTMX SSE Extension Requires Explicit Include
**What goes wrong:** `hx-ext="sse"` silently does nothing if the HTMX SSE extension script is not loaded. The HTMX core (~14KB) does NOT include the SSE extension.
**Why it happens:** HTMX extensions are modular and loaded separately.
**How to avoid:** Embed and serve both `htmx.min.js` (core) and `htmx-sse.js` (SSE extension) as separate static files. Both are available on the HTMX CDN; download and embed them both.

---

## Code Examples

### Complete Web State and Router Setup

```rust
// src/web/mod.rs
use std::sync::Arc;
use axum::{Router, routing::get};
use tokio_util::sync::CancellationToken;

pub struct WebState {
    /// Read-only access to supervisor watch channels.
    /// The handles themselves are owned by the daemon loop via DaemonState.
    /// We take a snapshot Vec of the names + Receivers only.
    pub server_handles: Arc<Vec<crate::supervisor::ServerHandle>>,
    pub log_agg: Arc<crate::logs::LogAggregator>,
}

pub fn build_router(state: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(routes::status_page))
        .route("/tools", get(routes::tools_page))
        .route("/logs", get(sse::logs_page))
        .route("/logs/stream", get(sse::log_stream_handler))
        .route("/partials/status", get(routes::status_partial))
        .route("/partials/tools/{server_name}", get(routes::tool_detail_partial))
        .route("/health", get(routes::health_handler))
        .route("/static/htmx.min.js", get(assets::serve_htmx))
        .route("/static/htmx-sse.js", get(assets::serve_htmx_sse))
        .route("/static/style.css", get(assets::serve_css))
        .with_state(state)
}

pub async fn start_web_server(
    port: u16,
    state: Arc<WebState>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await
        .map_err(|e| anyhow::anyhow!("Failed to bind web UI on port {port}: {e}"))?;

    tracing::info!("Web UI listening on http://{addr}");

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}
```

### SSE Log Stream with Filter

```rust
// src/web/sse.rs
use axum::{extract::{Query, State}, response::sse::{Event, KeepAlive, Sse}};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use futures_util::stream::Stream;
use std::{convert::Infallible, sync::Arc};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LogParams {
    pub server: Option<String>,
}

pub async fn log_stream_handler(
    State(state): State<Arc<super::WebState>>,
    Query(params): Query<LogParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_agg.subscribe();
    let filter_server = params.server;

    let stream = BroadcastStream::new(rx)
        .filter_map(move |result| {
            let filter = filter_server.clone();
            async move {
                let line = result.ok()?;  // drop lagged errors
                if let Some(ref name) = filter {
                    if &line.server != name {
                        return None;
                    }
                }
                let data = format!("[{}] {} {}", line.server, format_ts(line.timestamp), line.message);
                Some(Ok(Event::default()
                    .event("log")
                    .data(data)))
            }
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn format_ts(t: std::time::SystemTime) -> String {
    // Reuse logs::format_system_time or inline the same algorithm
    crate::logs::format_system_time_pub(t)
}
```

### /health Endpoint

```rust
// src/web/routes.rs (health handler)
use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub servers: Vec<ServerHealthEntry>,
}

#[derive(Serialize)]
pub struct ServerHealthEntry {
    pub name: String,
    pub process_state: String,
    pub health: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
}

pub async fn health_handler(
    State(state): State<Arc<super::WebState>>,
) -> Json<HealthResponse> {
    let servers: Vec<ServerHealthEntry> = state.server_handles.iter()
        .map(|h| {
            let snap = h.state_rx.borrow().clone();  // non-blocking
            ServerHealthEntry {
                name: h.name.clone(),
                process_state: snap.process_state.to_string(),
                health: snap.health.to_string(),
                pid: snap.pid,
                restart_count: snap.restart_count,
            }
        })
        .collect();

    let overall = derive_overall_status(&servers);
    Json(HealthResponse { status: overall, servers })
}

fn derive_overall_status(servers: &[ServerHealthEntry]) -> String {
    if servers.is_empty() {
        return "healthy".to_string();
    }
    let has_failed = servers.iter().any(|s| s.health == "failed");
    let has_degraded = servers.iter().any(|s| s.health.starts_with("degraded"));
    if has_failed { "failed".to_string() }
    else if has_degraded { "degraded".to_string() }
    else { "healthy".to_string() }
}
```

### Askama Template Setup

```rust
// src/web/routes.rs (status page)
use askama::Template;
use askama_web::WebTemplate;

#[derive(Template, WebTemplate)]
#[template(path = "status.html")]
pub struct StatusPage {
    pub servers: Vec<ServerCardData>,
    pub healthy_count: usize,
    pub total_count: usize,
}

pub struct ServerCardData {
    pub name: String,
    pub state: String,         // "running", "stopped", "fatal", etc.
    pub health: String,        // "healthy", "degraded(2 missed)", etc.
    pub pid: String,           // formatted or "-"
    pub uptime: String,        // "01:23:45" or "-"
    pub restart_count: u32,
    pub tool_count: String,    // "3T/1R/0P" or "-"
    pub transport: String,
}

impl ServerCardData {
    pub fn from_snapshot(name: String, snap: &crate::types::ServerSnapshot) -> Self {
        let caps = &snap.capabilities;
        let tool_count = if caps.introspected_at.is_some() {
            format!("{}T/{}R/{}P", caps.tools.len(), caps.resources.len(), caps.prompts.len())
        } else {
            "-".to_string()
        };
        ServerCardData {
            name,
            state: snap.process_state.to_string(),
            health: snap.health.to_string(),
            pid: snap.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
            uptime: snap.uptime_since
                .map(|s| crate::types::format_uptime(s.elapsed()))
                .unwrap_or_else(|| "-".to_string()),
            restart_count: snap.restart_count,
            tool_count,
            transport: snap.transport.clone(),
        }
    }
}
```

Template file `templates/status.html`:
```html
{% extends "base.html" %}
{% block content %}
<div id="server-cards"
     hx-get="/partials/status"
     hx-trigger="every 3s"
     hx-swap="innerHTML">
  {% for server in servers %}
  <div class="server-card">
    <div class="card-header">
      <span class="status-dot status-{{ server.state }}"></span>
      <strong class="server-name">{{ server.name }}</strong>
      <span class="health-badge health-{{ server.health }}">{{ server.health }}</span>
    </div>
    <div class="card-body">
      <span>PID: {{ server.pid }}</span>
      <span>Uptime: {{ server.uptime }}</span>
      <span>Restarts: {{ server.restart_count }}</span>
      <span>Tools: {{ server.tool_count }}</span>
    </div>
  </div>
  {% endfor %}
</div>
{% endblock %}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `askama_axum` crate for IntoResponse | `askama_web` with `"axum-0.8"` feature | askama 0.13 (2024) | STACK.md cites 0.12.x which predates this change; must use `askama_web` |
| `/:param` path syntax in axum | `/{param}` path syntax | axum 0.8 (Jan 2025) | Any routes with path params must use new syntax |
| `#[async_trait]` on custom extractors | Removed, native async trait | axum 0.8 | No impact for Phase 4 (no custom extractors needed) |

**Deprecated:**
- `askama_axum` crate: removed at askama 0.13, not maintained. Do NOT add it as a dependency.
- axum `/:param` route syntax: does not work in 0.8. Use `/{param}`.

---

## Open Questions

1. **Handle ownership model for web server**
   - What we know: `DaemonState.handles` is `Arc<Mutex<Vec<ServerHandle>>>` (needs write access for restart commands). The web server only needs read access to watch channels.
   - What's unclear: Should `WebState` hold a clone of `Arc<Mutex<Vec<ServerHandle>>>` (and lock briefly to read)? Or extract a `Vec<Arc<watch::Receiver<ServerSnapshot>>>` at startup?
   - Recommendation: Simplest approach — `WebState` holds `Arc<Mutex<Vec<ServerHandle>>>` cloned from `DaemonState`. Lock briefly, snapshot watch channels, unlock. Avoids any new types.

2. **`format_system_time` visibility**
   - What we know: `logs::format_system_time` is currently private (not pub).
   - What's unclear: The SSE handler needs it for log line formatting.
   - Recommendation: Either make it `pub(crate)` in `logs.rs`, or duplicate the logic in `web/sse.rs`. Making it `pub(crate)` is cleaner.

3. **Web UI in foreground mode vs daemon mode**
   - What we know: Context says "web server runs inside the same process as the daemon/supervisor." The current `main.rs` has two separate code paths: foreground loop and daemon loop.
   - What's unclear: Should the web server also start in foreground mode?
   - Recommendation: Yes — start the web server in both modes (D-13 says it starts even with no browser open). Spawn it from the foreground loop path too, using the same `CancellationToken`.

---

## Environment Availability

Step 2.6: SKIPPED — Phase 4 adds code/config to an existing Rust project. No new external tools, databases, or CLIs are required. All new dependencies (`axum`, `askama`, `askama_web`, `tokio-stream`) are fetched via `cargo` at build time. HTMX and its SSE extension are downloaded as static files and embedded at compile time — no network access required at runtime.

The project builds with the existing `cargo` toolchain (stable Rust, edition 2021). No additional environment setup needed.

---

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Rust built-in `#[test]` + `tokio::test` + `assert_cmd` |
| Config file | none (uses `[dev-dependencies]` in Cargo.toml) |
| Quick run command | `cargo test --lib -- web` |
| Full suite command | `cargo test` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| WEB-01 | Web server starts on configured port (default 3456) | integration | `cargo test --test web_ui web_server_starts_on_default_port` | ❌ Wave 0 |
| WEB-01 | Port is configurable via `[hub] web_port` in TOML | unit | `cargo test --lib -- config::tests::hub_config_web_port` | ❌ Wave 0 |
| WEB-02 | GET / returns 200 with all server cards | integration | `cargo test --test web_ui status_page_returns_200` | ❌ Wave 0 |
| WEB-02 | Status partial returns card grid fragment only | integration | `cargo test --test web_ui status_partial_returns_fragment` | ❌ Wave 0 |
| WEB-03 | GET /tools returns 200 with accordion sections | integration | `cargo test --test web_ui tools_page_returns_200` | ❌ Wave 0 |
| WEB-04 | GET /logs/stream returns SSE content-type | integration | `cargo test --test web_ui log_stream_returns_sse_content_type` | ❌ Wave 0 |
| WEB-04 | SSE stream filters by server name via query param | integration | `cargo test --test web_ui log_stream_filters_by_server` | ❌ Wave 0 |
| WEB-05 | GET /health returns JSON 200 with status and servers array | integration | `cargo test --test web_ui health_returns_json` | ❌ Wave 0 |
| WEB-05 | /health responds in <100ms with 10 servers | integration | `cargo test --test web_ui health_responds_under_100ms` | ❌ Wave 0 |

### Sampling Rate
- **Per task commit:** `cargo test --lib -- web`
- **Per wave merge:** `cargo test`
- **Phase gate:** Full suite green before `/gsd:verify-work`

### Wave 0 Gaps
- [ ] `tests/web_ui.rs` — covers WEB-01 through WEB-05 (all web integration tests)
- [ ] `portpicker` dev-dependency — for finding free test ports (already in STACK.md but not in Cargo.toml)
- [ ] `static/htmx.min.js` + `static/htmx-sse.js` — download and commit these files (Wave 0 prerequisite for compilation)
- [ ] `templates/` directory with `base.html`, `status.html`, `tools.html`, `logs.html` stubs — required for `cargo build` to succeed once askama derives are added

*(Existing test infrastructure: 15 test files in `tests/`, using `assert_cmd` + `tempfile`. Pattern is well established.)*

---

## Project Constraints (from CLAUDE.md)

The following directives from `./CLAUDE.md` apply directly to Phase 4 planning:

| Directive | Applies To |
|-----------|-----------|
| No `unwrap()` in production code — use `?` or handle | All axum handlers, SSE stream, server start |
| No `println!` debug output — use `tracing` macros | Web server startup, request logging |
| `///` rustdoc on all public items | `WebState`, `start_web_server`, all route handlers |
| No JS framework — server-rendered HTML only | Confirmed by CONTEXT.md D-03 (HTMX only) |
| `cargo fmt` before every commit | All new `.rs` files |
| `cargo clippy -- -D warnings` must pass | All new code |
| `tokio` for all async — no blocking calls on runtime | Route handlers, SSE stream |
| `anyhow` for binary error handling | `start_web_server`, `main.rs` web spawn |
| Pattern match exhaustively | `ProcessState` match in template data builders |

---

## Sources

### Primary (HIGH confidence)
- axum 0.8.8 official docs (docs.rs) — SSE handler, State extractor, `with_graceful_shutdown`
- askama_web 0.15.2 docs (docs.rs) — WebTemplate derive, `"axum-0.8"` feature
- tokio-stream BroadcastStream docs — `BroadcastStream::new(rx)` pattern
- crates.io API — verified current versions of axum (0.8.8), askama (0.15.6), tower-http (0.6.8)

### Secondary (MEDIUM confidence)
- tokio.rs blog post "Announcing axum 0.8.0" (Jan 2025) — confirmed breaking changes (path syntax, Option<T> extractor, async_trait removal)
- axum GitHub discussions #2565 — `CancellationToken` + `with_graceful_shutdown` pattern confirmed working
- HTMX official docs — `hx-trigger="every 3s"` polling, `hx-ext="sse"` + `sse-swap` confirmed
- WebSearch results — `askama_axum` removal at 0.13 confirmed by multiple community sources

### Tertiary (LOW confidence)
- WebSearch single-source claims about HTMX SSE reconnection behavior — needs validation during implementation

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — verified crate versions against crates.io; askama_web migration confirmed
- Architecture: HIGH — based on existing codebase patterns (watch channels, CancellationToken, LogAggregator already in place)
- Pitfalls: HIGH — watch::Ref and BroadcastStream::Lagged are documented Rust async gotchas; path syntax change confirmed in axum 0.8 announcement
- Template/HTMX patterns: MEDIUM — HTMX polling and SSE docs confirmed, but specific filter pill reconnect interaction needs testing

**Research date:** 2026-04-06
**Valid until:** 2026-07-06 (stable ecosystem; axum 0.8 is recent; askama_web is actively maintained)
