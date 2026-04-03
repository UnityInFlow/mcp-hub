# Phase 1: Config & Process Supervisor - Context

**Gathered:** 2026-04-03
**Status:** Ready for planning

<domain>
## Phase Boundary

Parse and validate TOML config, then spawn/stop/restart child processes with correct signal handling and exponential backoff auto-restart. Foreground mode only (daemon mode is Phase 3). No health checks, no MCP introspection, no web UI — just process lifecycle management.

Requirements: CFG-01, CFG-02, PROC-01, PROC-02, PROC-03, PROC-05, PROC-06, PROC-07, PROC-08, PROC-09, DMN-01.

</domain>

<decisions>
## Implementation Decisions

### TOML config shape
- **D-01:** Config file named `mcp-hub.toml`
- **D-02:** Global config at `~/.config/mcp-hub/mcp-hub.toml`, local config in current directory. Local servers merge into global. Local overrides global for same server name.
- **D-03:** Servers defined as `[servers.<name>]` map (name is the TOML key, not a field). Example: `[servers.mcp-github]`
- **D-04:** Per-server fields: `command` (required), `args` (optional array), `env` (optional inline table), `env_file` (optional path, values override inline env), `transport` (optional, defaults to "stdio"), `cwd` (optional)
- **D-05:** Per-server health/restart overrides allowed: `health_check_interval`, `max_retries`, `restart_delay` — these are optional, Phase 2 sets global defaults
- **D-06:** Config validation on load: invalid TOML = clear error and non-zero exit. Unknown fields = warning (forward-compatible).

### Shutdown behavior
- **D-07:** SIGTERM first, wait 5 seconds, then SIGKILL. Fixed timeout (not configurable in v1).
- **D-08:** Kill entire process group, not just direct child. Use `process_group(0)` on spawn to isolate children from hub's signal group.
- **D-09:** Parallel stop: send SIGTERM to all servers simultaneously, then wait for all. No ordering.
- **D-10:** Ctrl+C in foreground: install `tokio::signal::ctrl_c()` handler, trigger ordered shutdown (SIGTERM all -> wait 5s -> SIGKILL survivors -> exit).

### Backoff & failure policy
- **D-11:** Exponential backoff: 1s -> 2s -> 4s -> 8s -> 16s -> 32s -> 60s (max). Add +/-30% jitter per interval.
- **D-12:** 10 consecutive failures = Fatal state. Server stops restarting. Requires manual `mcp-hub restart <name>` to clear.
- **D-13:** Backoff counter resets to 0 after server has been Running for 60 seconds continuously.
- **D-14:** Fatal state clears on fresh `mcp-hub start` — assumes user may have fixed the config/environment.

### CLI output format
- **D-15:** `mcp-hub start` waits for all servers to launch, then prints a status table (name, state, PID).
- **D-16:** Colors enabled by default (green=running, red=failed, yellow=starting). `--no-color` flag disables. Auto-detect if stdout is a TTY.
- **D-17:** Quiet by default: only errors and final status table. `-v` for verbose (show start/stop events), `-vv` for debug (show process spawn details).

### Claude's Discretion
- Table formatting library choice (e.g., comfy-table, tabled, manual)
- Exact color palette and styling
- Internal channel/mpsc architecture for process supervision
- Error message wording and formatting
- Config file search order implementation details

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` -- Full feature spec, key features checklist, technical stack, implementation todos
- `.planning/PROJECT.md` -- Project vision, constraints, key decisions

### Research findings
- `.planning/research/STACK.md` -- Recommended crate versions (tokio, clap, serde, toml, nix, etc.)
- `.planning/research/ARCHITECTURE.md` -- Component breakdown, data flow, process supervisor design
- `.planning/research/PITFALLS.md` -- Critical pitfalls: zombie processes, pipe blocking, signal races, blocking I/O

### Ecosystem constraints
- `CLAUDE.md` -- Rust coding standards (no unwrap, exhaustive pattern matching, cargo clippy/fmt)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- None — greenfield project, no existing code

### Established Patterns
- None — first Rust code in this repo. Follow patterns from `.planning/research/ARCHITECTURE.md`.

### Integration Points
- TOML config schema designed in this phase is consumed by every subsequent phase
- Process supervisor state machine is extended in Phase 2 (health monitoring) and Phase 3 (MCP introspection)
- CLI subcommand structure established here is extended in every subsequent phase

</code_context>

<specifics>
## Specific Ideas

- Config merge behavior inspired by Docker Compose (local overrides global)
- Server name as TOML key (not a separate field) — like Cargo workspaces
- Start output should feel like `docker compose up` — clean status table after launch
- Quiet-by-default like modern CLIs (ripgrep, fd) — noise on demand, not by default

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 01-config-process-supervisor*
*Context gathered: 2026-04-03*
