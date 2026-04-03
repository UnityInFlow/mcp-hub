# Research Summary — mcp-hub

## Stack

**Core:** Rust stable + Tokio (full features) + clap 4 (derive) + serde/serde_json + toml 0.8 + anyhow/thiserror + tracing/tracing-subscriber.

**Web:** Axum 0.8 + askama 0.12 (compile-time templates) + tower-http 0.6. No JavaScript framework.

**Process management:** tokio::process + nix 0.29 (Unix process groups, signals). daemonize 0.5 for daemon mode. Unix domain socket for daemon IPC.

**MCP protocol:** Custom thin JSON-RPC 2.0 client (~100 lines). No MCP SDK needed — just `initialize`, `tools/list`, `resources/list`, `prompts/list`, `ping`.

**Testing:** tokio::test + assert_cmd + predicates + tempfile + portpicker.

## Table Stakes Features

These are expected by anyone who has used PM2/supervisord:

1. **TOML config file** — declarative server definitions (name, command, args, env, transport)
2. **Start/stop/restart/status** — per-server and all-at-once
3. **Auto-restart on crash** — exponential backoff with jitter, ceiling at 60s, give-up after N failures
4. **Unified log streaming** — `mcp-hub logs --follow`, per-server filter, timestamp + source prefix
5. **Health monitoring** — periodic checks with clear state machine (Starting -> Running -> Healthy -> Degraded -> Failed -> Stopped)
6. **Graceful shutdown** — SIGTERM then SIGKILL, drain logs before exit

## Differentiating Features

These are MCP-specific and set mcp-hub apart:

1. **MCP protocol introspection** — enumerate tools, resources, prompts per server via JSON-RPC
2. **Claude Code config generator** — output correct `settings.json` mcpServers block
3. **Cursor config generator** — output Cursor MCP config snippet
4. **Config gen dual mode** — offline from TOML, or --live from introspected running state
5. **Web UI with tools browser** — see all servers, their health, and what tools they expose
6. **Transport-aware** — explicit stdio vs HTTP transport in config, different health check paths

## Anti-Features (Do NOT Build)

- Remote server management / SSH deploys
- Plugin/module system
- Cluster mode / load balancing
- Container orchestration
- Real-time metrics/APM (beyond basic health status)

## Architecture (Component Build Order)

1. **Config Loader** — TOML parsing, validation (no async, no deps)
2. **Process Supervisor** — spawn, lifecycle state machine, stdin/stdout pipe management
3. **Log Aggregator** — stderr capture, ring buffer, broadcast to subscribers
4. **MCP Client** — JSON-RPC over stdio, request ID correlation, introspection calls
5. **Hub State** — central state store, RwLock, aggregates supervisor + MCP + health data
6. **Health Monitor** — periodic MCP ping, state transitions, backoff calculation
7. **CLI** — clap subcommands, foreground vs daemon mode, IPC
8. **Web UI** — Axum routes, askama templates, SSE log streaming
9. **Config Generator** — TOML-only and --live modes, Claude Code + Cursor output

## Critical Pitfalls to Address

| # | Pitfall | When |
|---|---------|------|
| 1 | Zombie processes from dropped Child handles | Process supervisor (phase 1) |
| 2 | stdin/stdout pipe buffer blocking Tokio runtime | Log aggregator (phase 1) |
| 3 | Backoff without jitter causes restart storms | Health monitor (phase 1) |
| 4 | Conflating process liveness with MCP health | Health state model (phase 1) |
| 5 | Ctrl+C signal races in foreground mode | Process supervisor (phase 1) |
| 7 | JSON-RPC ID correlation for concurrent introspection | MCP client (phase 3) |
| 8 | stdio vs HTTP transport confusion | Config schema (phase 1) |
| 9 | PID files alone aren't enough for daemon IPC | Daemon mode (phase 2) |
| 10 | Cross-platform process management assumptions | All phases |
| 11 | Web UI blocked by health check mutex | Web UI (phase 4) |
| 12 | Config generator producing stale tool lists | Config gen (phase 5) |

---

*Synthesized: 2026-04-03*
