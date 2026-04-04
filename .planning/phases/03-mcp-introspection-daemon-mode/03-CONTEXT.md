# Phase 3: MCP Introspection & Daemon Mode - Context

**Gathered:** 2026-04-04
**Status:** Ready for planning

<domain>
## Phase Boundary

Implement full MCP capability discovery (initialize + tools/list + resources/list + prompts/list with concurrent JSON-RPC ID correlation), add background daemon mode with Unix socket IPC, wire all CLI subcommands (status/stop/restart/logs) to daemon, and add config reload via SIGHUP.

Requirements: MCP-01, MCP-02, MCP-03, MCP-04, CFG-03, DMN-02, DMN-03, DMN-04, DMN-05.

</domain>

<decisions>
## Implementation Decisions

### Introspection flow
- **D-01:** Introspect once when server first reaches Healthy status. Re-introspect on server restart. No periodic re-introspection in v1.
- **D-02:** Send initialize + tools/list + resources/list + prompts/list CONCURRENTLY per server. Use HashMap<RequestId, oneshot::Sender<JsonRpcResponse>> dispatcher for JSON-RPC ID correlation (Pitfall #7). Each server introspected independently (parallel across servers).
- **D-03:** Store introspection results (tools, resources, prompts) directly in ServerSnapshot. Counts and full lists available to status table and future web UI.
- **D-04:** Transport type explicit in config (D-04 Phase 1). v1 implements stdio transport only (MCP-04).

### Daemon architecture
- **D-05:** Unix domain socket for daemon IPC. Newline-delimited JSON protocol. Reuse serde types from internal state.
- **D-06:** PID file alongside socket — belt and suspenders. Socket is the liveness check, PID file for stale cleanup after crash.
- **D-07:** Duplicate daemon prevention: on startup with --daemon, attempt to connect to socket. If connectable, exit with clear error. If stale socket (PID dead), clean up and start.
- **D-08:** Socket path and PID path: Claude's discretion (recommend ~/.config/mcp-hub/ for XDG compliance).

### Config reload (SIGHUP)
- **D-09:** Derive PartialEq on ServerConfig. On SIGHUP, reload TOML, compare each server old vs new via PartialEq.
- **D-10:** Unchanged servers: skip (keep running, no restart).
- **D-11:** Changed servers: stop old, start new with updated config.
- **D-12:** New servers (in new config, not in old): start automatically.
- **D-13:** Removed servers (in old config, not in new): stop gracefully. Config is the source of truth.

### CLI command wiring
- **D-14:** CLI commands (status/stop/restart/logs) connect to daemon Unix socket. Claude's discretion on timeout (recommend 5s for simple commands, 10s for operations that trigger introspection).
- **D-15:** If socket not connectable, print "No daemon running. Use `mcp-hub start --daemon`" and exit 1. No fallback to foreground detection.

### Claude's Discretion
- Exact socket path within ~/.config/mcp-hub/
- IPC message schema (request/response JSON structure)
- CLI timeout values per command type
- Whether to use `daemonize` crate or manual fork
- Request ID allocation strategy for concurrent introspection
- How introspection interacts with health check loop (share stdin/stdout or serialize access)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` -- Feature spec, MCP introspection approach, daemon mode design
- `.planning/PROJECT.md` -- Project vision, validated requirements from Phase 1+2

### Research findings
- `.planning/research/ARCHITECTURE.md` -- Component 4 (MCP Client), Component 7 (CLI), Component 8 (daemon IPC)
- `.planning/research/PITFALLS.md` -- Pitfall #7 (JSON-RPC ID correlation), Pitfall #8 (stdio vs HTTP transport), Pitfall #9 (daemon IPC via socket not just PID)
- `.planning/research/STACK.md` -- daemonize crate, tokio::net::UnixListener

### Prior phase context
- `.planning/phases/01-config-process-supervisor/01-CONTEXT.md` -- D-01 through D-06 (config shape, server fields)
- `.planning/phases/02-health-monitoring-logging/02-CONTEXT.md` -- D-12 (dedicated MCP client task), D-15 (ping JSON-RPC format)

### Existing code
- `src/mcp/health.rs` -- ping_server and run_health_check_loop (extend for introspection)
- `src/mcp/protocol.rs` -- PingRequest, JsonRpcResponse (extend for tools/list etc.)
- `src/supervisor.rs` -- SpawnedProcess with stdin/stdout, ServerHandle, start_all_servers
- `src/types.rs` -- ServerSnapshot, HealthStatus, ProcessState, ServerConfig
- `src/main.rs` -- foreground loop, CLI dispatch, current Phase 3 stubs for stop/restart/status/logs
- `src/cli.rs` -- Current Commands enum with Start/Stop/Restart/Status/Logs

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/mcp/protocol.rs` -- PingRequest/JsonRpcResponse to extend with InitializeRequest, ToolsListRequest, etc.
- `src/mcp/health.rs` -- ping_server pattern (write JSON to stdin, read from stdout with timeout) to extend for introspection
- `ServerSnapshot` watch channel -- add McpCapabilities field (tools/resources/prompts)
- `src/supervisor.rs` start_all_servers -- extend to trigger introspection after Healthy
- Phase 3 stubs in main.rs -- Commands::Stop/Restart/Status/Logs all exit 1 with "daemon mode" message, replace with socket client

### Established Patterns
- Per-server tokio tasks with CancellationToken
- mpsc channels for supervisor commands
- watch channels for state broadcasting
- JSON-RPC over stdin/stdout (ping_server pattern)
- Newline-delimited protocol (already used for MCP ping)

### Integration Points
- Health check loop needs to coordinate with introspection (share or serialize stdin/stdout access)
- Daemon mode replaces the foreground loop (run_foreground_loop) with a socket listener
- CLI commands switch from direct state access to socket client
- Config reload (SIGHUP) needs access to running ServerHandles to diff and restart

</code_context>

<specifics>
## Specific Ideas

- Concurrent introspection with HashMap<id, oneshot> dispatcher is the key architectural piece — gets reused by future web UI API
- Daemon IPC protocol should be simple enough to test with socat/netcat
- Config reload should feel like nginx's `kill -HUP` — seamless, no downtime for unchanged servers

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-mcp-introspection-daemon-mode*
*Context gathered: 2026-04-04*
