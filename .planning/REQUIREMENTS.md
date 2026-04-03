# Requirements: mcp-hub

**Defined:** 2026-04-03
**Core Value:** Developers running 5+ MCP servers can manage them all from one place — one config, one command, one log stream.

## v1 Requirements

### Configuration

- [ ] **CFG-01**: User can define MCP servers in a TOML config file (name, command, args, env, transport type)
- [ ] **CFG-02**: Config validates on load with clear error messages for invalid entries
- [ ] **CFG-03**: User can reload config without restarting the hub (SIGHUP or command)

### Process Lifecycle

- [ ] **PROC-01**: User can start all configured servers with `mcp-hub start`
- [ ] **PROC-02**: User can stop all servers with `mcp-hub stop`
- [ ] **PROC-03**: User can restart a specific server by name with `mcp-hub restart <name>`
- [ ] **PROC-04**: User can view status of all servers with `mcp-hub status` (name, state, PID, uptime, restarts)
- [ ] **PROC-05**: Servers auto-restart on crash with exponential backoff (1s -> 2s -> 4s -> max 60s) with jitter
- [ ] **PROC-06**: After N consecutive failures, server is marked Fatal and stops restarting
- [ ] **PROC-07**: Hub performs graceful shutdown on Ctrl+C: SIGTERM to all children, wait up to 5s, then SIGKILL
- [ ] **PROC-08**: Child processes are spawned in separate process groups to avoid signal races
- [ ] **PROC-09**: No zombie processes after stop — all child handles properly awaited/killed

### Health Monitoring

- [ ] **HLTH-01**: Hub performs periodic health checks on each server at configurable intervals
- [ ] **HLTH-02**: Health checks use MCP protocol ping (JSON-RPC), not just process liveness
- [ ] **HLTH-03**: Servers have distinct health states: Starting, Running, Healthy, Degraded, Failed, Stopped
- [ ] **HLTH-04**: Health check timeout (default 5s) prevents slow servers from blocking others
- [ ] **HLTH-05**: Degraded state triggers after N consecutive missed pings before transitioning to Failed

### Logging

- [ ] **LOG-01**: User can stream unified logs from all servers with `mcp-hub logs --follow`
- [ ] **LOG-02**: User can filter logs to a specific server with `mcp-hub logs --server <name>`
- [ ] **LOG-03**: Each log line is prefixed with timestamp and server name
- [ ] **LOG-04**: Logs are captured from stderr (stdout is reserved for MCP protocol)
- [ ] **LOG-05**: In-memory ring buffer stores last N lines per server (configurable, default 10000)

### MCP Introspection

- [ ] **MCP-01**: Hub introspects each server on startup: initialize, tools/list, resources/list, prompts/list
- [ ] **MCP-02**: JSON-RPC request IDs are properly correlated for concurrent introspection calls
- [ ] **MCP-03**: Introspection results are stored and available via status/web UI
- [ ] **MCP-04**: Transport type (stdio/HTTP) is explicit in config; v1 implements stdio transport

### Config Generation

- [ ] **GEN-01**: User can generate Claude Code settings.json mcpServers block with `mcp-hub gen-config --format claude`
- [ ] **GEN-02**: User can generate Cursor MCP config snippet with `mcp-hub gen-config --format cursor`
- [ ] **GEN-03**: Default mode generates from TOML config (offline, no servers need to be running)
- [ ] **GEN-04**: `--live` flag generates from running introspected state including tool/resource/prompt lists
- [ ] **GEN-05**: Generated config includes a timestamp and mcp-hub version comment

### Web UI

- [ ] **WEB-01**: Hub serves a web UI on a configurable port (default 3456)
- [ ] **WEB-02**: Status page shows all servers with name, state, PID, uptime, restart count, health, tool count
- [ ] **WEB-03**: Tools browser page shows tools/resources/prompts per server
- [ ] **WEB-04**: Log viewer streams logs via SSE (all servers or filtered by server)
- [ ] **WEB-05**: Health endpoint at /health returns JSON for external monitoring

### Daemon Mode

- [ ] **DMN-01**: User can run hub in foreground (default) — runs in terminal, Ctrl+C stops everything
- [ ] **DMN-02**: User can run hub as background daemon with `mcp-hub start --daemon`
- [ ] **DMN-03**: Daemon communicates via Unix domain socket (not just PID file)
- [ ] **DMN-04**: `mcp-hub stop` connects to daemon socket and triggers graceful shutdown
- [ ] **DMN-05**: Multiple daemon instances prevented by socket liveness check

### Setup Wizard

- [ ] **WIZ-01**: User can add a new server interactively with `mcp-hub init`
- [ ] **WIZ-02**: Wizard prompts for name, command, args, env vars, transport type
- [ ] **WIZ-03**: Wizard appends to existing TOML config or creates a new one

### Distribution

- [ ] **DIST-01**: Pre-built binaries for macOS (arm64, x86_64), Linux (x86_64, aarch64), Windows (x86_64)
- [ ] **DIST-02**: Homebrew formula: `brew install unityinflow/tap/mcp-hub`
- [ ] **DIST-03**: Installable via `cargo install mcp-hub`

## v2 Requirements

### Enhanced Transport

- **TRANS-01**: HTTP/SSE transport support for remote MCP servers
- **TRANS-02**: Transport auto-detection from server behavior

### Advanced Management

- **ADV-01**: Server dependency ordering (start A before B)
- **ADV-02**: Watch mode — restart on config file change
- **ADV-03**: Per-server resource limits (memory, CPU)
- **ADV-04**: Log file persistence with rotation

### Ecosystem

- **ECO-01**: Windsurf config generator
- **ECO-02**: VS Code MCP extension config generator
- **ECO-03**: Export to Procfile/docker-compose format

## Out of Scope

| Feature | Reason |
|---------|--------|
| Remote server management / SSH | Local-only tool, complexity too high for v1 |
| MCP server implementation | This manages servers, doesn't provide them |
| GUI desktop app | Web UI from binary is sufficient |
| Plugin/module system | Direct TOML config, unnecessary abstraction for v1 |
| Container/Docker orchestration | Docker Compose already exists for this |
| Real-time APM/metrics | Beyond scope — basic health status is enough |
| Cluster mode / load balancing | MCP servers are not stateless web servers |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| CFG-01 | Phase 1 | Pending |
| CFG-02 | Phase 1 | Pending |
| CFG-03 | Phase 3 | Pending |
| PROC-01 | Phase 1 | Pending |
| PROC-02 | Phase 1 | Pending |
| PROC-03 | Phase 1 | Pending |
| PROC-04 | Phase 2 | Pending |
| PROC-05 | Phase 1 | Pending |
| PROC-06 | Phase 1 | Pending |
| PROC-07 | Phase 1 | Pending |
| PROC-08 | Phase 1 | Pending |
| PROC-09 | Phase 1 | Pending |
| HLTH-01 | Phase 2 | Pending |
| HLTH-02 | Phase 2 | Pending |
| HLTH-03 | Phase 2 | Pending |
| HLTH-04 | Phase 2 | Pending |
| HLTH-05 | Phase 2 | Pending |
| LOG-01 | Phase 2 | Pending |
| LOG-02 | Phase 2 | Pending |
| LOG-03 | Phase 2 | Pending |
| LOG-04 | Phase 2 | Pending |
| LOG-05 | Phase 2 | Pending |
| MCP-01 | Phase 3 | Pending |
| MCP-02 | Phase 3 | Pending |
| MCP-03 | Phase 3 | Pending |
| MCP-04 | Phase 3 | Pending |
| GEN-01 | Phase 5 | Pending |
| GEN-02 | Phase 5 | Pending |
| GEN-03 | Phase 5 | Pending |
| GEN-04 | Phase 5 | Pending |
| GEN-05 | Phase 5 | Pending |
| WEB-01 | Phase 4 | Pending |
| WEB-02 | Phase 4 | Pending |
| WEB-03 | Phase 4 | Pending |
| WEB-04 | Phase 4 | Pending |
| WEB-05 | Phase 4 | Pending |
| DMN-01 | Phase 1 | Pending |
| DMN-02 | Phase 3 | Pending |
| DMN-03 | Phase 3 | Pending |
| DMN-04 | Phase 3 | Pending |
| DMN-05 | Phase 3 | Pending |
| WIZ-01 | Phase 6 | Pending |
| WIZ-02 | Phase 6 | Pending |
| WIZ-03 | Phase 6 | Pending |
| DIST-01 | Phase 7 | Pending |
| DIST-02 | Phase 7 | Pending |
| DIST-03 | Phase 7 | Pending |

**Coverage:**
- v1 requirements: 42 total
- Mapped to phases: 42
- Unmapped: 0

---
*Requirements defined: 2026-04-03*
*Last updated: 2026-04-03 after initial definition*
