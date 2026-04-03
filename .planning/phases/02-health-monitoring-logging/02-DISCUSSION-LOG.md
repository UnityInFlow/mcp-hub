# Phase 2: Health Monitoring & Logging - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.

**Date:** 2026-04-03
**Phase:** 2-health-monitoring-logging
**Areas discussed:** Health state model, Log aggregation design, Status command output, MCP ping mechanism

---

## Health State Model

### State model architecture

| Option | Description | Selected |
|--------|-------------|----------|
| Extend ProcessState | Add Healthy/Degraded to existing enum | |
| Separate HealthStatus | New enum alongside ProcessState | ✓ |
| You decide | | |

**User's choice:** Separate HealthStatus

### Degraded threshold

| Option | Description | Selected |
|--------|-------------|----------|
| 2 consecutive misses | Quick detection | ✓ |
| 3 consecutive misses | More tolerant | |
| You decide | | |

**User's choice:** 2 consecutive misses

### Failed health threshold

| Option | Description | Selected |
|--------|-------------|----------|
| 5 consecutive | ~2.5 min total at 30s | ✓ |
| 3 consecutive | ~3 min total, faster | |
| You decide | | |

**User's choice:** 5 consecutive (total 7 missed pings)

---

## Log Aggregation Design

### Ring buffer size

| Option | Description | Selected |
|--------|-------------|----------|
| 10,000 lines | Matches spec default | ✓ |
| 1,000 lines | Lower memory | |
| Configurable | Default 10k, override in TOML | |

**User's choice:** 10,000 lines

### Log access method

| Option | Description | Selected |
|--------|-------------|----------|
| New CLI subcommand | Separate process, needs daemon IPC for live | ✓ |
| Inline in foreground | stdin command | |
| Both | stdin + CLI subcommand | |

**User's choice:** New CLI subcommand

### Log format

| Option | Description | Selected |
|--------|-------------|----------|
| Timestamp + name + message | [2026-04-03T12:34:56] [mcp-github] ... | |
| Colored name prefix | mcp-github \| Server started... (docker-compose style) | ✓ |
| You decide | | |

**User's choice:** Colored name prefix

---

## Status Command Output

### Columns

| Option | Description | Selected |
|--------|-------------|----------|
| Full table | Name, Process State, Health, PID, Uptime, Restarts, Transport | ✓ |
| Compact table | Name, State (combined), PID, Uptime, Restarts | |
| You decide | | |

**User's choice:** Full table

### Uptime format

| Option | Description | Selected |
|--------|-------------|----------|
| Human-friendly | "2h 15m" | |
| Precise | "02:15:30" | ✓ |
| You decide | | |

**User's choice:** Precise (HH:MM:SS)

---

## MCP Ping Mechanism

### Ping method

| Option | Description | Selected |
|--------|-------------|----------|
| Dedicated MCP client task | Separate task per server | ✓ |
| Shared ping loop | Single sequential task | |
| You decide | | |

**User's choice:** Dedicated MCP client task

### Ping interval

| Option | Description | Selected |
|--------|-------------|----------|
| 30 seconds | Matches spec | ✓ |
| 15 seconds | More responsive | |
| 60 seconds | Conservative | |

**User's choice:** 30 seconds

### Ping timeout

| Option | Description | Selected |
|--------|-------------|----------|
| 5 seconds | Matches HLTH-04 | ✓ |
| 3 seconds | Tighter | |
| You decide | | |

**User's choice:** 5 seconds

---

## Claude's Discretion

- Ring buffer implementation details
- Color assignment for server prefixes
- JSON-RPC ID management
- stdout sharing between ping and future introspection
- Log timestamp precision

## Deferred Ideas

None
