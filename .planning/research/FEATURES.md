# FEATURES.md — mcp-hub Process Manager Research

> Research dimension: What features do process managers and MCP tooling have?
> Goal: Identify table stakes vs differentiators vs anti-features for an MCP-specific process manager.

---

## Reference Systems Surveyed

| System | Category | Primary Use Case |
|--------|----------|-----------------|
| PM2 | Node.js process manager | Long-running Node apps, cluster mode, production deploys |
| systemd | Linux init/service manager | System services, socket activation, dependency ordering |
| supervisord | Python process manager | Simple cross-platform daemon supervision |
| Foreman / Overmind | Procfile runners | Dev-time multi-process startup from Procfile |
| Docker Compose | Container orchestration | Multi-container dev environments |
| Hivemind | Procfile runner (Go) | Dev-time, tmux-based, minimal |
| mcp-proxy (existing) | MCP-specific | SSE-to-stdio bridging, not a process manager |
| Claude Code built-in MCP config | MCP-specific | Static JSON config, no lifecycle management |

---

## Feature Inventory by System

### PM2 — Key Features

**Lifecycle management**
- `start`, `stop`, `restart`, `delete`, `reload` (zero-downtime reload)
- Named process identifiers and numeric IDs
- `pm2 list` — tabular status view with PID, uptime, restarts, memory, CPU
- `pm2 save` / `pm2 resurrect` — persist process list across reboots
- `pm2 startup` — generates init script (systemd/upstart/launchd/rc.d)

**Health and resilience**
- Auto-restart on crash
- `max_restarts` config per process
- `min_uptime` threshold (crash loop detection — if dies within N ms, don't restart)
- Exponential backoff is NOT native in PM2 — it uses a fixed restart delay
- Memory limit restart (`max_memory_restart`)
- Watch mode — restart on file change (dev use)
- Online/stopped/errored status states

**Logging**
- Per-process log files (stdout + stderr to separate files)
- `pm2 logs` — tail all logs interleaved
- `pm2 logs <name>` — tail single process
- `pm2 flush` — clear log files
- Log rotation via pm2-logrotate module
- Timestamp prefixes on log lines

**Configuration**
- Ecosystem file (`ecosystem.config.js` or `.json`) — declarative process definitions
- Per-process: name, script, args, env, cwd, instances, exec_mode
- Environment-specific configs (`env_production`, `env_staging`)
- Cluster mode — spin up N workers, auto load-balance

**Monitoring and observability**
- `pm2 monit` — real-time dashboard in terminal (CPU, memory per process)
- `pm2 plus` (paid) — web dashboard, alerts, server metrics
- Metrics exposed via keymetrics.io API
- `pm2 describe <name>` — detailed process info dump

**Ecosystem integration**
- Module system (pm2-logrotate, pm2-auto-pull, pm2-slack)
- Deploy system (`pm2 deploy`) — SSH-based deploys
- Docker support (`pm2-docker` entrypoint)

---

### systemd — Key Features

**Lifecycle management**
- `start`, `stop`, `restart`, `reload`, `enable`, `disable`, `mask`
- Unit files (`.service`, `.socket`, `.timer`, `.target`, `.path`)
- `systemctl status` — state, PID, recent log lines, exit code

**Health and resilience**
- `Restart=on-failure | always | on-abnormal | no`
- `RestartSec=` — fixed delay between restarts
- `StartLimitIntervalSec` + `StartLimitBurst` — crash loop detection with hard stop
- `WatchdogSec` — process must send keepalive via sd_notify or gets killed
- `TimeoutStartSec` / `TimeoutStopSec`

**Dependency ordering**
- `After=`, `Before=`, `Requires=`, `Wants=`, `PartOf=` — fine-grained DAG
- `BindsTo=` — if dependency dies, this dies too
- Target units as synchronisation barriers

**Logging**
- All stdout/stderr captured to journald automatically
- `journalctl -u <unit>` — query logs for a unit
- `journalctl -f` — follow mode
- Structured logging (key=value pairs)
- Log retention policies, log rotation built-in

**Resource control (cgroups)**
- `CPUQuota=`, `MemoryMax=`, `TasksMax=`
- Process isolation via cgroups v2
- OOM killer integration

**Socket activation**
- Service starts on-demand when a socket receives a connection
- Zero-downtime service restarts via socket hand-off

**Security**
- `User=`, `Group=`
- Namespace isolation: `PrivateTmp=`, `PrivateNetwork=`, `ProtectSystem=`
- Capability dropping: `CapabilityBoundingSet=`
- Seccomp filters: `SystemCallFilter=`

---

### supervisord — Key Features

**Lifecycle management**
- `start`, `stop`, `restart`, `status` via `supervisorctl`
- Program groups — manage related processes together
- Priority ordering within groups
- `supervisord` daemon + `supervisorctl` client separated

**Health and resilience**
- Auto-restart: `autorestart=true|false|unexpected`
- `startsecs` — process must stay up N seconds to be considered "started"
- `startretries` — max restart attempts before giving up (FATAL state)
- `exitcodes` — which exit codes count as expected (no restart)
- No exponential backoff — fixed retry count

**Logging**
- `stdout_logfile`, `stderr_logfile` per process
- `stdout_logfile_maxbytes` + `stdout_logfile_backups` — built-in rotation
- `supervisord.log` for daemon itself

**Configuration**
- INI-style config file (`supervisord.conf`)
- `[program:name]` sections
- `[group:name]` for grouping
- `[include]` directives for splitting config across files
- Environment variable injection: `environment=KEY="val",KEY2="val2"`

**RPC interface**
- XML-RPC API over HTTP — `supervisorctl` uses this
- Third-party web UIs built on top of this (Cesi, supervisor-ui)
- Event listeners — subscribe to process state change events

**Notable limitations vs PM2**
- No cluster mode
- No built-in metrics
- No deploy system
- INI config is less expressive than JSON/YAML

---

### Procfile Runners (Foreman, Overmind, Hivemind)

**Core model**
- `Procfile` — one line per process: `web: node server.js`
- `foreman start` — starts all processes, Ctrl+C stops all
- Color-coded interleaved output in terminal
- No daemon mode in basic Foreman

**Overmind (Go, tmux-based)**
- Each process in its own tmux pane — attach to debug
- `overmind connect <name>` — attach to a process's terminal
- `overmind restart <name>` — restart single process
- Socket-based control

**Hivemind (Go, minimal)**
- Subset of Overmind, no tmux dependency
- Purely foreground, no control socket

**Key gap**: No health checks, no MCP awareness, no config generation.

---

## MCP-Specific Tooling (Current State, April 2026)

### Claude Code built-in MCP config (`settings.json`)
- Static JSON block under `mcpServers` key
- Each entry: `command`, `args`, `env`, `type` (stdio/sse)
- Claude Code manages the process itself — starts on demand, kills when done
- No persistent daemon — processes restart per-session
- No introspection surfaced to the user
- No health monitoring — if server crashes mid-session, error surfaced as tool failure
- No unified log view

### mcp-proxy
- Bridges stdio MCP servers to SSE (HTTP) transport
- Lets multiple clients share one server instance
- No lifecycle management, no config generation, no health checks
- Single-server focused — not a multi-server manager

### Inspector (MCP official debugging tool)
- Browser-based interactive debugger
- Connect to a running MCP server and call tools/list, resources/list manually
- No process management — requires the server to already be running
- Dev tool, not a production manager

### MCP Get / mcpm (community package managers)
- `mcpm` — install MCP servers from a registry, similar to npm for MCP
- Handles installation and updates, not runtime management
- Does not start/stop/monitor processes

**Gap summary**: Nothing in the MCP ecosystem manages process lifecycle + health + logs + introspection as a unified tool. mcp-hub fills this gap.

---

## Feature Taxonomy for mcp-hub

### TABLE STAKES
> Must have or users will use PM2 directly (worse DX but known tool).

| Feature | Complexity | What it must do | Why it's table stakes |
|---------|-----------|-----------------|----------------------|
| TOML config file | Low | `[[servers]]` blocks with name, command, args, env, cwd | Without config-as-code, it's worse than a shell script |
| `start` / `stop` / `restart` / `status` | Medium | Full lifecycle for all servers and individual ones | The whole product premise — without this, nothing works |
| Auto-restart on crash | Medium | Detect non-zero exit, restart with exponential backoff | MCP servers crash; silent death kills the AI session |
| Crash loop detection | Low | If a server restarts N times in M seconds, mark FAILED, stop retrying | Prevents runaway restart loops consuming CPU |
| Unified log streaming | Medium | `mcp-hub logs --follow` — interleaved, timestamped, color-coded by server | Without this, users open 5 separate terminals |
| Per-server log files | Low | Write each server's stdout/stderr to `~/.mcp-hub/logs/<name>.log` | Persistent — users need to inspect after crashes |
| Foreground mode | Low | Default: runs in terminal, Ctrl+C stops all | Simplest usage path; matches Procfile runner UX |
| Daemon mode | Medium | `--daemon` flag, communicates via Unix socket | Required for always-on use (startup scripts, persistent dev env) |
| Exit codes | Low | 0=all healthy, 1=one or more servers failed/crashed | Scripts and CI depend on this |
| Single binary, zero deps | Low | Compile to static binary; no Node, Python, or JVM needed | The Rust bet — if it requires a runtime, use PM2 |

**Dependency chain**: Config parsing -> Process spawning -> Health loop -> Log aggregation. Each depends on the previous.

---

### DIFFERENTIATORS
> Competitive advantage specific to the MCP use case. PM2/supervisord cannot do these.

| Feature | Complexity | What it does | Why it differentiates |
|---------|-----------|--------------|----------------------|
| MCP introspection (tools/list) | High | After startup, run MCP `initialize` + `tools/list` JSON-RPC over stdio; cache result | Shows users what each server actually provides; PM2 knows nothing about the protocol |
| MCP introspection (resources + prompts) | High | `resources/list` + `prompts/list` per server; structured capability index | Complete picture of server capabilities — no other tool shows this |
| Claude Code config generator | Low | Emit correct `mcpServers` JSON block for `settings.json` | Eliminates manual config — direct path to value |
| Cursor config generator | Low | Emit Cursor's MCP config format | Same premise; second most popular target IDE |
| Config gen `--live` mode | Medium | Pull from running introspection, not just TOML — catches runtime discrepancies | Accurate config even when the server's capabilities differ from what the TOML documents |
| `mcp-hub init` wizard | Medium | Interactive prompts: name, command, args, env; writes to `mcp.toml`; tests the connection | Reduces setup friction from ~10 minutes to ~1 minute for new users |
| Web UI with capability browser | High | Axum server, status page, tools/resources/prompts per server, log viewer | Visual surface for introspection — no MCP tool has this |
| Health check via MCP ping | Medium | Heartbeat using MCP `ping` or keepalive JSON-RPC call, not just process-is-alive | Process-alive is insufficient — a frozen MCP server will not serve requests |
| Transport type support (stdio + SSE) | Medium | Manage both stdio-transport servers (spawned process) and SSE-transport servers (remote URL) | SSE servers can't be "started" but can be health-checked; no other manager handles both |
| Server dependency ordering | Medium | `depends_on = ["filesystem"]` in TOML; start in topological order | Some MCP servers depend on others (e.g., a memory server before an agent server) |

**Dependency chain**: MCP introspection is a prerequisite for the web capability browser and `--live` config gen. `mcp-hub init` depends on introspection to validate the connection. Server dependency ordering is independent.

---

### NICE-TO-HAVE (v0.2+, not v0.0.1)

| Feature | Complexity | Notes |
|---------|-----------|-------|
| Log rotation | Low | Supervisord-style: `max_bytes` + `max_backups` per log file |
| Resource limits (memory/CPU cap) | High | Requires cgroup integration on Linux — not worth it for v0 |
| Metrics export (Prometheus endpoint) | Medium | `/metrics` on the web UI server; process uptime, restart count, tool call count |
| Tool call counting / observability | High | Intercept JSON-RPC to count tool invocations; integrates with token-dashboard (tool 06) |
| Named profiles | Low | `mcp-hub start --profile work` vs `--profile personal` — different server sets |
| `mcp-hub validate` | Low | Parse config, check commands exist, but don't start — CI use case |
| Shell completion (zsh/bash/fish) | Low | `clap` generates this for free; just needs wiring |
| `--format json` output on status/logs | Low | Machine-readable output for scripting |
| Server versioning / update notifications | High | Check if a newer version of the underlying MCP server is available |
| Multi-user support | Very High | Non-goal for local tool |

---

### ANTI-FEATURES
> Deliberately NOT building these. Complexity cost exceeds value for this tool's scope.

| Anti-Feature | Why Not |
|-------------|---------|
| Remote server management (SSH) | Out of scope by design; adds auth complexity, security surface; mcp-hub is local-only |
| MCP server implementation | This manages servers, doesn't provide them; conflates two jobs |
| GUI desktop app (Electron/Tauri) | Web UI served from the binary is sufficient; no install complexity |
| Plugin system | TOML config handles customization; plugins add semver hell without clear benefit for v1 |
| Cloud deployment / remote agents | Local developer tool; cloud is a different product |
| Cluster mode (N replicas of one server) | MCP servers are stateful stdio processes; N replicas would require a load balancer and shared state — fundamentally different architecture |
| PM2-style deploy system (`pm2 deploy`) | SSH-based deploy is orthogonal to MCP management; out of scope |
| Auto-update of mcp-hub itself | Complex, security-sensitive; homebrew/cargo handles updates |
| Log shipping (Datadog, Splunk) | Observability at this level is overkill for a local dev tool |
| Windows Service integration | Pre-built binaries run on Windows; OS-level service registration is complexity without demand signal |
| mcp-hub Plus / paid tier | Open source only for v1; monitization decision is future |
| Process groups / program groups (supervisord-style) | Dependency ordering (`depends_on`) covers the same need more explicitly |

---

## Feature Dependency Map

```
TOML config parsing
  └─> Process spawning (tokio::process)
        ├─> Health check loop (30s interval)
        │     └─> Auto-restart with exponential backoff
        │           └─> Crash loop detection
        ├─> Log aggregation (stdout/stderr from child processes)
        │     └─> Log streaming (`mcp-hub logs --follow`)
        │     └─> Log files on disk
        └─> MCP introspection (initialize + tools/list + resources/list + prompts/list)
              ├─> Web UI capability browser
              ├─> Config generator --live mode
              └─> `mcp-hub init` connection validation

Daemon mode (Unix socket)
  └─> IPC protocol (start/stop/restart/status commands over socket)
        └─> `mcp-hub` CLI as client to daemon

Server dependency ordering (`depends_on`)
  └─> Topological sort at startup
        └─> Process spawning (ordered)
```

**Critical path to v0.0.1**: TOML -> Process spawning -> Health loop -> Log aggregation -> Foreground mode -> MCP introspection -> Config generators -> `mcp-hub init` -> Daemon mode -> Web UI.

---

## Competitive Positioning Summary

| Capability | PM2 | supervisord | systemd | mcp-hub |
|-----------|-----|-------------|---------|---------|
| Zero runtime deps | No (Node) | No (Python) | No (Linux-only) | Yes (Rust binary) |
| TOML config | No | No | No | Yes |
| MCP introspection | No | No | No | Yes |
| Config gen for Claude/Cursor | No | No | No | Yes |
| Web capability browser | Paid only | 3rd party | No | Yes |
| MCP health check (protocol-level) | No | No | No | Yes |
| Cross-platform | Partial | Partial | No | Yes |
| Exponential backoff | No | No | Yes | Yes |
| Crash loop detection | Partial | Yes | Yes | Yes |
| Unified log stream | Yes | No | journald | Yes |
| Daemon mode | Yes | Yes | N/A | Yes |
| Interactive setup wizard | No | No | No | Yes |
| Server dependency ordering | No | Yes (priority) | Yes (full DAG) | Yes (depends_on) |

**The MCP-specific column is empty for all incumbents.** That is the moat.

---

*Research date: 2026-04-02*
*Sources: PM2 documentation (pm2.keymetrics.io), supervisord documentation (supervisord.org), systemd man pages (systemd.io), MCP specification (modelcontextprotocol.io), direct tooling survey of mcp-proxy, mcpm, MCP Inspector.*
