# Phase 1: Config & Process Supervisor - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-03
**Phase:** 1-config-process-supervisor
**Areas discussed:** TOML config shape, Shutdown behavior, Backoff & failure policy, CLI output format

---

## TOML Config Shape

### Config path

| Option | Description | Selected |
|--------|-------------|----------|
| Local mcp-hub.toml | Look in current directory first | |
| ~/.config/mcp-hub/ | XDG-style global config | |
| Both with merge | Global default + local override | ✓ |

**User's choice:** Both with merge
**Notes:** Local servers merge into global. Local overrides global for same server name.

### Env vars

| Option | Description | Selected |
|--------|-------------|----------|
| Inline in TOML | env = { API_KEY = "abc" } | |
| Reference .env file | env_file = ".env" | |
| Both supported | Inline env + optional env_file | ✓ |

**User's choice:** "what is your suggestion" -> Claude recommended Both supported
**Notes:** User deferred to Claude's recommendation. env_file values override inline, matches Docker Compose convention.

### Server table

| Option | Description | Selected |
|--------|-------------|----------|
| [[server]] array | Repeated TOML array sections | |
| [servers.name] map | Name as TOML key | ✓ |

**User's choice:** "suggestion" -> Claude recommended [servers.name] map
**Notes:** User deferred to Claude. Cleaner, prevents duplicate names at TOML level.

### Transport default

| Option | Description | Selected |
|--------|-------------|----------|
| Default stdio | Optional, defaults to "stdio" | ✓ |
| Always explicit | Required field | |

**User's choice:** "suggestion" -> Claude recommended Default stdio
**Notes:** User deferred to Claude. 95%+ MCP servers use stdio.

### Config directory (follow-up)

| Option | Description | Selected |
|--------|-------------|----------|
| ~/.config/mcp-hub/ | XDG standard | ✓ |
| ~/.mcp-hub/ | Simpler dot-directory | |
| You decide | | |

**User's choice:** ~/.config/mcp-hub/

### Config file name (follow-up)

| Option | Description | Selected |
|--------|-------------|----------|
| mcp-hub.toml | Clear, specific | ✓ |
| config.toml | Generic | |
| hub.toml | Short | |

**User's choice:** mcp-hub.toml

### Extra fields (follow-up)

| Option | Description | Selected |
|--------|-------------|----------|
| Minimal | command, args, env, env_file, transport, cwd | |
| With health config | Also health_check_interval, max_retries, restart_delay | ✓ |

**User's choice:** With health config

---

## Shutdown Behavior

### SIGTERM wait

| Option | Description | Selected |
|--------|-------------|----------|
| 5 seconds | Matches Docker default | ✓ |
| 10 seconds | More generous | |
| Configurable per-server | Default 5s, override in TOML | |

**User's choice:** 5 seconds

### Kill scope

| Option | Description | Selected |
|--------|-------------|----------|
| Process group | Kill entire group | ✓ |
| Direct child only | Simpler but risks orphans | |
| You decide | | |

**User's choice:** Process group

### Shutdown order

| Option | Description | Selected |
|--------|-------------|----------|
| Parallel stop | SIGTERM to all at once | ✓ |
| Reverse order | Stop last-started first | |
| You decide | | |

**User's choice:** Parallel stop

---

## Backoff & Failure Policy

### Max retries

| Option | Description | Selected |
|--------|-------------|----------|
| 10 attempts | ~5 min of retrying with backoff | ✓ |
| 5 attempts | Fail fast | |
| Configurable per-server | Default 10, override in TOML | |

**User's choice:** 10 attempts

### Stable reset

| Option | Description | Selected |
|--------|-------------|----------|
| After 60s stable | Reset retry count after 60s running | ��� |
| After 30s stable | Shorter, more forgiving | |
| You decide | | |

**User's choice:** After 60s stable

### Fatal servers on restart

| Option | Description | Selected |
|--------|-------------|----------|
| Retry on fresh start | Fatal clears when hub restarts | ✓ |
| Stay Fatal until manual reset | Require explicit restart command | |
| You decide | | |

**User's choice:** Retry on fresh start

---

## CLI Output Format

### Start output

| Option | Description | Selected |
|--------|-------------|----------|
| One line per server | Simple, like systemd | |
| Table after all started | Wait, then print table | ✓ |
| Progressive table | Update in-place | |

**User's choice:** Table after all started

### Colors

| Option | Description | Selected |
|--------|-------------|----------|
| Yes with --no-color | Colors by default, flag to disable | ✓ |
| No colors default | Plain text by default | |
| You decide | | |

**User's choice:** Yes with --no-color

### Verbosity

| Option | Description | Selected |
|--------|-------------|----------|
| Quiet by default | Only errors and final status, -v/-vv | ✓ |
| Moderate default | Show start/stop events | |
| You decide | | |

**User's choice:** Quiet by default

---

## Claude's Discretion

- Table formatting library choice
- Exact color palette and styling
- Internal channel/mpsc architecture
- Error message wording
- Config file search order implementation details

## Deferred Ideas

None — discussion stayed within phase scope
