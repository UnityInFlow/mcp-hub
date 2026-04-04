# Phase 3: MCP Introspection & Daemon Mode - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.

**Date:** 2026-04-04
**Phase:** 3-mcp-introspection-daemon-mode
**Areas discussed:** Introspection flow, Daemon architecture, Config reload (SIGHUP), CLI command wiring

---

## Introspection Flow

### When to introspect

| Option | Description | Selected |
|--------|-------------|----------|
| Startup + on demand | Once at Healthy, then on explicit command | |
| Startup + periodic | On Healthy, then every N minutes | |
| Startup only | Once when Healthy, re-introspect on restart | ✓ |

### Concurrency

| Option | Description | Selected |
|--------|-------------|----------|
| Sequential per server | Send one at a time, wait each | |
| Concurrent per server | All 4 requests at once, ID correlation | ✓ |
| You decide | | |

### Storage

| Option | Description | Selected |
|--------|-------------|----------|
| In ServerSnapshot | Add tools/resources/prompts to snapshot | ✓ |
| Separate McpCapabilities | New struct alongside snapshot | |
| You decide | | |

---

## Daemon Architecture

### Socket path

| Option | Description | Selected |
|--------|-------------|----------|
| ~/.config/mcp-hub/mcp-hub.sock | XDG-compliant | |
| /tmp/mcp-hub.sock | Temp location | |
| You decide | Claude picks | ✓ |

### IPC protocol

| Option | Description | Selected |
|--------|-------------|----------|
| Newline-delimited JSON | Simple, debuggable with socat | ✓ |
| Length-prefixed binary | More robust for large payloads | |
| You decide | | |

### PID file

| Option | Description | Selected |
|--------|-------------|----------|
| Yes — belt and suspenders | Socket + PID file | ✓ |
| Socket only | | |
| You decide | | |

---

## Config Reload (SIGHUP)

### Diff strategy

| Option | Description | Selected |
|--------|-------------|----------|
| Full struct comparison | PartialEq on ServerConfig | ✓ |
| Hash comparison | | |
| You decide | | |

### New servers

| Option | Description | Selected |
|--------|-------------|----------|
| Start automatically | Start immediately on reload | ✓ |
| Require explicit start | | |
| You decide | | |

### Removed servers

| Option | Description | Selected |
|--------|-------------|----------|
| Stop gracefully | Config is source of truth | ✓ |
| Keep running | | |
| You decide | | |

---

## CLI Command Wiring

### Detection method

| Option | Description | Selected |
|--------|-------------|----------|
| Try socket, fail fast | Connect or exit 1 | |
| Auto-detect mode | Check daemon then foreground | |
| You decide | Claude picks | ✓ |

### Timeout

| Option | Description | Selected |
|--------|-------------|----------|
| 5 seconds | | |
| 10 seconds | | |
| You decide | Claude picks per command type | ✓ |

---

## Claude's Discretion

- Socket path, IPC message schema, CLI timeouts, daemonize approach, request ID strategy, introspection/health coordination

## Deferred Ideas

None
