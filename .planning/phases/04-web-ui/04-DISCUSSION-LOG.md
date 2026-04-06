# Phase 4: Web UI - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md -- this log preserves the alternatives considered.

**Date:** 2026-04-06
**Phase:** 04-web-ui
**Areas discussed:** Visual style, Dashboard layout, Tools browser UX, Log viewer behavior

---

## Visual Style

### Overall aesthetic

| Option | Description | Selected |
|--------|-------------|----------|
| Utilitarian terminal | Monospace, dark bg, minimal styling. Like htop/PM2. | |
| Clean modern | System font, light bg, subtle cards, accent colors. Like Tailwind UI/Linear. | ✓ |
| Hybrid | Dark mode with modern layout. Like GitHub dark/Vercel. | |

**User's choice:** Clean modern
**Notes:** None

### Client-side interactivity

| Option | Description | Selected |
|--------|-------------|----------|
| Zero JS | Pure server-rendered HTML + CSS. Page refreshes for updates. | |
| Minimal vanilla JS | Small inline scripts for auto-refresh, scroll control, toggles. | |
| HTMX | htmx for partial page updates. SPA-like but server-rendered. | ✓ |

**User's choice:** HTMX
**Notes:** None

### HTMX delivery

| Option | Description | Selected |
|--------|-------------|----------|
| Embedded | Bundle htmx.min.js inside the binary. Works offline. | ✓ |
| CDN | Load from unpkg/cdnjs. Requires internet. | |

**User's choice:** Embedded
**Notes:** None

### Theme

| Option | Description | Selected |
|--------|-------------|----------|
| Light only | Single light theme. Simpler CSS. | ✓ |
| System-follows | prefers-color-scheme media query. Doubles CSS work. | |
| Dark only | Dark theme only. May clash with clean modern direction. | |

**User's choice:** Light only
**Notes:** None

---

## Dashboard Layout

### Server presentation

| Option | Description | Selected |
|--------|-------------|----------|
| Card grid | One card per server in responsive grid. Click to expand/navigate. | ✓ |
| Table | Traditional table like PM2 list/docker ps. Compact rows. | |
| Hybrid | Summary cards at top, detailed table below. | |

**User's choice:** Card grid
**Notes:** None

### Refresh behavior

| Option | Description | Selected |
|--------|-------------|----------|
| Auto-refresh | HTMX polls every 2-5s, swaps card content in-place. | ✓ |
| Manual refresh | User clicks refresh or reloads page. | |

**User's choice:** Auto-refresh
**Notes:** None

### Page structure/navigation

| Option | Description | Selected |
|--------|-------------|----------|
| Tab bar | Top nav with Status/Tools/Logs tabs. HTMX loads content. | ✓ |
| Sidebar nav | Left sidebar with links. More room but heavier. | |
| Single page | All sections on one scrollable page. | |

**User's choice:** Tab bar
**Notes:** None

---

## Tools Browser UX

### Organization

| Option | Description | Selected |
|--------|-------------|----------|
| Per-server accordion | Collapsible server sections. HTMX lazy-loads on expand. | ✓ |
| Unified flat list | All tools in one table with server name column. | |
| Three-column layout | Server list / type tabs / detail panel. | |

**User's choice:** Per-server accordion
**Notes:** None

### Detail level per tool

| Option | Description | Selected |
|--------|-------------|----------|
| Name + description | Show tool name and description. Schema on click later. | ✓ |
| Full schema | Name, description, AND input schema inline. Visually heavy. | |
| Name only | Just tool names. Minimal. | |

**User's choice:** Name + description
**Notes:** None

---

## Log Viewer Behavior

### Streaming mechanism

| Option | Description | Selected |
|--------|-------------|----------|
| SSE + HTMX swap | hx-ext="sse" with sse-swap. Auto-appends lines. | ✓ |
| SSE + vanilla JS | Raw EventSource with custom script. More control. | |
| Polling | HTMX polls /api/logs every 1-2s. Simpler but laggy. | |

**User's choice:** SSE + HTMX swap
**Notes:** None

### Server filtering

| Option | Description | Selected |
|--------|-------------|----------|
| Clickable server pills | Row of colored pills at top. Click to toggle on/off. | ✓ |
| Dropdown select | Single dropdown for All/specific server. | |
| You decide | Claude picks best approach. | |

**User's choice:** Clickable server pills
**Notes:** None

### Visual style

| Option | Description | Selected |
|--------|-------------|----------|
| Terminal-like | Dark bg, monospace, colored prefixes. Like docker-compose. | ✓ |
| Web-native | Light bg, system font, alternating rows. | |

**User's choice:** Terminal-like
**Notes:** Intentional contrast with the rest of the clean modern UI.

---

## Claude's Discretion

- Template engine choice (askama vs tera vs manual)
- CSS approach (embedded stylesheet vs utility classes)
- Exact auto-refresh polling interval (2-5s range)
- Card click behavior (navigate vs expand)
- SSE event format and reconnection
- Static asset embedding (include_str! vs rust-embed)
- Empty state design
- Web server integration with daemon mode

## Deferred Ideas

None -- discussion stayed within phase scope
