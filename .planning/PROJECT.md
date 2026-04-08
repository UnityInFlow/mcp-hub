# mcp-hub

## What This Is

A local MCP server process manager — PM2 for MCP servers. Single Rust binary that manages the lifecycle of multiple MCP servers (start/stop/restart/status), monitors health with auto-restart on crash, streams unified logs, introspects server capabilities via MCP protocol, and generates config snippets for Claude Code and Cursor. Zero runtime dependencies, instant startup.

## Core Value

Developers running 5+ MCP servers can manage them all from one place — one config file, one command, one log stream — instead of copy-pasting startup commands across terminals.

## Requirements

### Validated

- TOML config file defining all MCP servers (name, command, args, env) — Validated in Phase 1
- Process lifecycle: start, stop, restart for all servers — Validated in Phase 1
- Auto-restart on crash with exponential backoff (1s -> 2s -> 4s -> max 60s) — Validated in Phase 1
- Foreground mode (default): runs in terminal, Ctrl+C stops everything — Validated in Phase 1
- Health monitoring: MCP ping checks with configurable interval, Healthy/Degraded/Failed states — Validated in Phase 2
- Unified log view: stderr capture with ring buffer, docker-compose style colored output — Validated in Phase 2
- Full MCP introspection: initialize + tools/list + resources/list + prompts/list with concurrent JSON-RPC ID correlation — Validated in Phase 3
- Daemon mode (--daemon flag): daemonizes with Unix socket IPC, PID file, duplicate prevention — Validated in Phase 3
- Config reload via SIGHUP: diff-based (unchanged=skip, changed=restart, new=start, removed=stop) — Validated in Phase 3
- Web UI: Axum + HTMX, status card grid, tools accordion, SSE log streaming, /health JSON endpoint — Validated in Phase 4
- Claude Code config generator: `gen-config --format claude` outputs mcpServers JSON block — Validated in Phase 5
- Cursor config generator: `gen-config --format cursor` outputs Cursor MCP config snippet — Validated in Phase 5
- Config gen modes: offline from TOML + `--live` from running introspected state with tool name comments — Validated in Phase 5

### Active
- [ ] Daemon mode (--daemon flag): daemonizes, communicates via socket/file
- [ ] `mcp-hub init`: interactive wizard to add a new server
- [ ] Pre-built binaries for macOS (arm64/x86_64), Linux (x86_64/aarch64), Windows
- [ ] Homebrew formula: `brew install unityinflow/tap/mcp-hub`

### Out of Scope

- Remote server management — local only, no SSH or network management
- MCP server implementation — this manages servers, doesn't provide them
- GUI desktop app — web UI served from the binary is sufficient
- Plugin system — direct TOML config, no plugin architecture needed for v1
- Cloud deployment — this is a local developer tool

## Context

- Tool 07 in the UnityInFlow AI agent tooling ecosystem (20 tools total)
- Phase 2 tool — 6 tools already shipped (spec-linter, ai-changelog, injection-scanner, spec-ci-plugin, budget-breaker, token-dashboard)
- MCP (Model Context Protocol) is becoming infrastructure for AI-assisted development; managing multiple servers is daily friction
- No established PM2-equivalent exists in the MCP ecosystem yet — first-mover opportunity
- Target audience: developers using Claude Code, Cursor, or similar AI tools with multiple MCP servers

## Constraints

- **Stack**: Rust stable (edition 2021) + Tokio async runtime — zero runtime dependencies, instant startup
- **CLI**: clap (derive feature) for argument parsing
- **Config**: TOML via toml crate — familiar to Rust/systems community
- **Web UI**: Axum + server-rendered HTML — no JavaScript framework, no npm required
- **Serialization**: serde + serde_json for all data structures
- **Error handling**: anyhow for binary, thiserror for library code
- **Quality**: cargo clippy -D warnings, cargo fmt, no unwrap() in production
- **CI**: Self-hosted runners (arc-runner-unityinflow for X64, orangepi for ARM64)
- **Distribution**: Pre-built binaries + Homebrew tap + cargo install

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust single binary | Zero runtime deps, instant startup, low memory — critical for always-on process manager | -- Pending |
| TOML over YAML/JSON | Familiar to Rust/systems community, human-readable, good for config files | -- Pending |
| Axum for web UI | Rust-native, no JS framework, works on any machine without npm | -- Pending |
| Both foreground + daemon modes | Foreground simpler for dev, daemon needed for production/always-on use | -- Pending |
| Full MCP introspection | tools + resources + prompts gives complete picture of each server's capabilities | -- Pending |
| Config gen: both TOML and live modes | Offline TOML for quick setup, --live for accurate introspected output | -- Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd:transition`):
1. Requirements invalidated? -> Move to Out of Scope with reason
2. Requirements validated? -> Move to Validated with phase reference
3. New requirements emerged? -> Add to Active
4. Decisions to log? -> Add to Key Decisions
5. "What This Is" still accurate? -> Update if drifted

**After each milestone** (via `/gsd:complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-04-08 after Phase 5 completion*
