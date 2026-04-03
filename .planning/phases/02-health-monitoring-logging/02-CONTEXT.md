# Phase 2: Health Monitoring & Logging - Context

**Gathered:** 2026-04-03
**Status:** Ready for planning

<domain>
## Phase Boundary

Add per-server health state machine with MCP protocol ping checks, and a unified log aggregator with ring buffer and streaming CLI. Extends the Phase 1 supervisor with health awareness and makes server logs accessible via `mcp-hub logs` and `mcp-hub status`.

Requirements: PROC-04, HLTH-01, HLTH-02, HLTH-03, HLTH-04, HLTH-05, LOG-01, LOG-02, LOG-03, LOG-04, LOG-05.

</domain>

<decisions>
## Implementation Decisions

### Health state model
- **D-01:** Health is a SEPARATE enum (`HealthStatus`) from `ProcessState`. Each server has both a process state and a health state. ProcessState tracks process lifecycle (Stopped/Starting/Running/Backoff/Fatal), HealthStatus tracks MCP-level health (Unknown/Healthy/Degraded/Failed).
- **D-02:** 2 consecutive missed pings = transition from Healthy to Degraded.
- **D-03:** 5 consecutive pings while Degraded = transition to Failed health. Total: 7 missed pings before Failed (~3.5 min at 30s interval).
- **D-04:** Health state resets to Unknown on server restart. Transitions to Healthy on first successful ping response.

### Log aggregation
- **D-05:** Ring buffer: 10,000 lines per server (default from REQUIREMENTS.md LOG-05). In-memory `VecDeque<LogLine>`.
- **D-06:** `mcp-hub logs` is a CLI subcommand (runs as separate process). For Phase 2, it dumps the ring buffer history from disk/state. Live streaming (`--follow`) needs daemon IPC (Phase 3 adds the socket). In foreground mode, logs are visible in the terminal output directly.
- **D-07:** Log line format: colored name prefix style — `mcp-github | Server started on port 3000`. Each server gets a distinct color (like docker-compose). Use the same owo-colors crate from Phase 1.
- **D-08:** Logs captured from stderr only (stdout reserved for MCP protocol). This is already implemented in Phase 1 supervisor — the log aggregator intercepts the stderr drain.

### Status command
- **D-09:** `mcp-hub status` table columns: Name | Process State | Health | PID | Uptime | Restarts | Transport. Full table with all relevant info.
- **D-10:** Uptime format: precise `HH:MM:SS` (hours:minutes:seconds).
- **D-11:** `mcp-hub status` in foreground mode reads from the running hub's state (stdin command). As a separate CLI invocation, it needs daemon IPC (Phase 3 stub with clear message).

### MCP ping mechanism
- **D-12:** Dedicated MCP client task per server. Separate tokio task owns the stdin writer + stdout reader for each server. Sends JSON-RPC `ping` request, reads response. This aligns with ARCHITECTURE.md recommendation.
- **D-13:** Default health check interval: 30 seconds. Configurable per-server via `health_check_interval` in TOML (already in ServerConfig from Phase 1 D-05).
- **D-14:** Ping timeout: 5 seconds per HLTH-04. Timeout does not block other servers' health checks (parallel tasks).
- **D-15:** MCP ping uses JSON-RPC 2.0: `{"jsonrpc":"2.0","method":"ping","id":1}`. Response expected within 5s. Non-response = missed ping.

### Claude's Discretion
- Ring buffer implementation details (mutex vs channel-based access)
- Color assignment algorithm for server name prefixes
- Exact JSON-RPC ID management for ping requests
- How to share stdout between MCP ping client and future Phase 3 introspection
- Log line timestamp precision (second vs millisecond)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` -- Full feature spec, health check interval (30s), MCP introspection approach
- `.planning/PROJECT.md` -- Project vision, validated Phase 1 requirements

### Research findings
- `.planning/research/STACK.md` -- Crate versions (tokio, serde_json for JSON-RPC)
- `.planning/research/ARCHITECTURE.md` -- Component 3 (Log Aggregator), Component 4 (MCP Client), Component 5 (Hub State) -- critical for Phase 2 design
- `.planning/research/PITFALLS.md` -- Pitfall #2 (pipe blocking), Pitfall #4 (conflating process liveness with MCP health), Pitfall #7 (JSON-RPC ID correlation), Pitfall #11 (web UI stale state -- relevant for state architecture)

### Phase 1 context
- `.planning/phases/01-config-process-supervisor/01-CONTEXT.md` -- D-05 (per-server health config fields), D-16/D-17 (color/verbosity patterns)

### Existing code
- `src/supervisor.rs` -- Current supervisor with SpawnedProcess.stdout handle (reserved for MCP client)
- `src/types.rs` -- ProcessState enum, BackoffConfig, ServerConfig (has health_check_interval field)
- `src/output.rs` -- Existing color/table output patterns to extend
- `src/main.rs` -- run_foreground_loop stdin command handler to extend with `logs` and `status`

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `SpawnedProcess.stdout: Option<ChildStdout>` — already stored for Phase 3 MCP client, now consumed by health check task
- `tokio::sync::watch` channels — used per-server for ProcessState broadcasting, extend for HealthStatus
- `print_status_table()` in output.rs — extend with Health, Uptime, Restarts, Transport columns
- `owo-colors` — already used for colored output, extend for log name prefixes
- `comfy-table` — already used for status table

### Established Patterns
- Per-server tokio tasks with CancellationToken for lifecycle management
- mpsc channels for supervisor commands (SupervisorCommand enum)
- watch channels for state broadcasting to output layer
- stderr drain via BufReader::lines() in supervisor — intercept point for log aggregator

### Integration Points
- Log aggregator intercepts stderr stream currently drained by supervisor
- MCP client takes ownership of SpawnedProcess.stdout handle
- Health check results broadcast via new watch channel or extend existing
- Status table in output.rs gains new columns from health + log data
- stdin command handler in main.rs gains `logs` and `status` commands

</code_context>

<specifics>
## Specific Ideas

- Log output format inspired by docker-compose: colored server name prefix with pipe separator
- Health state model separates "is the process alive?" from "is the MCP server responding?" — two distinct questions
- Status table inspired by PM2's `pm2 list` — comprehensive at a glance

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 02-health-monitoring-logging*
*Context gathered: 2026-04-03*
