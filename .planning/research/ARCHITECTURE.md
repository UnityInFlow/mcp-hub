# mcp-hub — Architecture Research

## How Process Managers Are Structured

Classic process managers (PM2, supervisord, systemd) share a common anatomy regardless of language:

### PM2 (Node.js)
- **Daemon process** — a long-running background process that owns all child processes
- **IPC bus** — PM2 uses an in-process event bus (axm) for inter-component messaging
- **Process wrapper** — each managed process gets a wrapper that captures stdout/stderr and forwards to the daemon
- **CLI client** — a thin client that serializes commands and sends them to the daemon over a Unix socket
- **Log aggregator** — the daemon merges all child log streams with timestamps and source labels
- **Resurrect/dump** — the daemon can serialize its process table to disk and reload it on startup

### supervisord (Python)
- **Single-process model** — the supervisor daemon runs all child processes as subprocesses; no separate client daemon
- **XML-RPC interface** — HTTP + XML-RPC for the `supervisorctl` client to communicate with the daemon
- **Event listeners** — processes can subscribe to supervisor events (PROCESS_STATE_RUNNING, CRASH, etc.)
- **Group management** — processes are organized into groups and can be controlled collectively
- **Log rotation built-in** — each process has a configured log file with rotation policy

### systemd (C)
- **Unit files** — declarative config describing what to run, dependencies, restart policy
- **cgroups integration** — each service gets a cgroup for resource accounting and isolation
- **D-Bus interface** — all control (start/stop/status) goes through D-Bus
- **Journal** — unified structured log sink (journald) that all services write to
- **Dependency graph** — services declare `After=`, `Requires=`, `Wants=` to establish ordering

### Common patterns across all three
1. A long-running supervisor process owns all child processes (parent PID matters for signal forwarding)
2. Child process I/O is captured at spawn time via pipe/pty redirection
3. Health checking is either passive (watch for exit) or active (periodic probe)
4. State transitions are explicit: STARTING → RUNNING → STOPPING → STOPPED → BACKOFF → FATAL
5. Configuration is declarative; the supervisor reconciles desired state against actual state
6. A control interface (socket/HTTP/D-Bus) separates the CLI client from the supervisor

---

## MCP-Specific Additions

MCP servers communicate via JSON-RPC 2.0 over stdio (or HTTP+SSE for remote servers). This means:

- **Transport is stdio** — the managed process's stdin/stdout are the MCP channel, not just log streams
- **Introspection is protocol-level** — to enumerate tools/resources/prompts, the hub must speak MCP itself, sending `initialize` + `tools/list` JSON-RPC requests on the child's stdin and reading responses from stdout
- **Health checks are protocol-level** — a ping is an MCP `ping` request, not a TCP connection check
- **Log capture must not contaminate the MCP channel** — stderr is for logs, stdout is for MCP protocol; these must be separated cleanly at spawn time

---

## Component Breakdown

### 1. Config Loader (`config/`)
**Responsibility:** Parse and validate `mcp-hub.toml`. Produce a typed `HubConfig` struct.

**Boundary:** Reads from disk only. No async. Called once at startup and on `SIGHUP` (reload).

**Data produced:** `HubConfig { servers: Vec<ServerConfig> }` where `ServerConfig` holds name, command, args, env, restart policy, health check interval.

**Dependencies:** `toml` crate, `serde`.

---

### 2. Process Supervisor (`supervisor/`)
**Responsibility:** Own the lifecycle of all child processes. Spawn, stop, restart. Maintain per-process state machine.

**Boundary:** Talks to the OS via `tokio::process::Command`. Exposes a message-passing API (mpsc channels) to the rest of the system. Does not know about MCP protocol.

**State machine per server:**
```
STOPPED -> STARTING -> RUNNING -> STOPPING -> STOPPED
                    -> BACKOFF  -> STARTING  (crash loop)
                    -> FATAL    (max retries exceeded)
```

**Backoff strategy:** Exponential: 1s → 2s → 4s → 8s → ... → max 60s. Reset after process has been RUNNING for >60s.

**Data produced:**
- `ProcessHandle { pid, stdin_tx, stdout_rx, stderr_rx, state }` per server
- `StateEvent { server_name, old_state, new_state, timestamp }` emitted to broadcast channel

**Dependencies:** `tokio::process`, `tokio::sync::mpsc`, `tokio::sync::broadcast`.

---

### 3. Log Aggregator (`logs/`)
**Responsibility:** Consume stderr streams from all processes, tag with server name + timestamp, store in a bounded ring buffer, stream to subscribers.

**Boundary:** Reads from `stderr_rx` channels provided by the supervisor. Writes to an in-memory ring buffer. Exposes a `subscribe()` method returning a `tokio::sync::broadcast::Receiver<LogLine>`.

**Why stderr only:** stdout is the MCP JSON-RPC channel and is owned by the MCP client layer.

**Data produced:** `LogLine { server: String, timestamp: DateTime<Utc>, level: Option<LogLevel>, message: String }`.

**Ring buffer size:** Configurable, default 10,000 lines per server.

**Dependencies:** `tokio::sync::broadcast`, `chrono` or `time`.

---

### 4. MCP Client (`mcp/`)
**Responsibility:** Speak the MCP JSON-RPC 2.0 protocol over a child process's stdin/stdout. Send `initialize`, `tools/list`, `resources/list`, `prompts/list`, and `ping`. Parse responses into typed structs.

**Boundary:** Takes a `stdin_tx: Sender<String>` and `stdout_rx: Receiver<String>` from the supervisor. Does not own the process. Exposes async methods: `initialize()`, `list_tools()`, `list_resources()`, `list_prompts()`, `ping()`.

**Key design decision:** stdout lines from child processes are MCP JSON-RPC. The supervisor must route stdout to the MCP client, not to the log aggregator.

**Data produced:**
- `McpCapabilities { tools: Vec<Tool>, resources: Vec<Resource>, prompts: Vec<Prompt> }`
- `HealthStatus { reachable: bool, latency_ms: Option<u64>, last_checked: DateTime<Utc> }`

**Dependencies:** `serde_json`, `tokio::sync::mpsc`.

---

### 5. Hub State (`state/`)
**Responsibility:** Single source of truth for all runtime state. Aggregates process states, MCP capabilities, and health statuses. Thread-safe via `Arc<RwLock<HubState>>`.

**Boundary:** Receives updates from the supervisor (state events), MCP client (capabilities, health), and log aggregator (log counts). Read by the web UI and CLI output layer. Never touches I/O directly.

**Data structure:**
```rust
pub struct HubState {
    pub servers: HashMap<String, ServerEntry>,
}

pub struct ServerEntry {
    pub config: ServerConfig,
    pub process_state: ProcessState,
    pub pid: Option<u32>,
    pub capabilities: Option<McpCapabilities>,
    pub health: HealthStatus,
    pub restart_count: u32,
    pub uptime_since: Option<DateTime<Utc>>,
}
```

**Dependencies:** `tokio::sync::RwLock`, `std::collections::HashMap`.

---

### 6. Web UI Server (`web/`)
**Responsibility:** Serve an Axum HTTP server with server-rendered HTML pages. Status page, tools browser, log viewer (SSE stream).

**Boundary:** Reads from `Arc<RwLock<HubState>>` for status/tools pages. Subscribes to the log aggregator's broadcast channel for the SSE log stream. Does not write to any other component.

**Routes:**
- `GET /` — server status table (name, state, PID, uptime, tool count, health)
- `GET /servers/:name/tools` — list tools for a specific server
- `GET /logs` — SSE stream of log lines (all servers interleaved)
- `GET /logs/:server` — SSE stream filtered to one server
- `GET /health` — JSON health endpoint for external monitoring

**Dependencies:** `axum`, `tokio::sync::RwLock`, SSE via `axum::response::sse`.

---

### 7. Config Generator (`codegen/`)
**Responsibility:** Produce Claude Code `settings.json` mcpServers block and Cursor config snippet from either the TOML config (offline) or live hub state (introspected).

**Boundary:** Takes either `HubConfig` (offline mode) or `Arc<RwLock<HubState>>` (live mode). Pure transformation — no I/O, just struct → String serialization.

**Modes:**
- `--from-config` — reads TOML, outputs config snippet without starting servers
- `--live` — reads running hub state including introspected capabilities

**Dependencies:** `serde_json`.

---

### 8. CLI (`cli/`)
**Responsibility:** Entry point. Parse subcommands with `clap`. Dispatch to the correct subsystem. Handle foreground vs. daemon mode.

**Subcommands:**
- `mcp-hub start [--daemon]` — start the hub (foreground or daemonized)
- `mcp-hub stop` — stop a running daemon (sends signal via PID file)
- `mcp-hub status` — print current state table (reads from socket/state file if daemon, or inline if foreground)
- `mcp-hub logs [--follow] [--server <name>]` — stream or dump logs
- `mcp-hub gen-config [--format claude|cursor] [--live]` — run the config generator
- `mcp-hub init` — interactive wizard to add a new server to TOML

**Daemon mode:** On `--daemon`, fork (or use `nix::unistd::daemon`) and write PID to `~/.mcp-hub/hub.pid`. The CLI client communicates with the daemon via a Unix socket at `~/.mcp-hub/control.sock`.

**Dependencies:** `clap` (derive), `anyhow`.

---

### 9. Control Socket (`control/`)
**Responsibility:** In daemon mode, listen on a Unix domain socket. Accept control commands from the CLI client (start server, stop server, get status JSON). This is the IPC layer between the CLI and the running daemon.

**Boundary:** Receives serialized commands from the CLI client. Dispatches to the supervisor or reads from hub state. Returns serialized responses.

**Protocol:** Newline-delimited JSON over Unix socket. Request: `{"cmd": "status"}`. Response: `{"ok": true, "data": {...}}`.

**Dependencies:** `tokio::net::UnixListener`, `serde_json`.

---

## Data Flow

```
mcp-hub.toml
     |
     v
[Config Loader] ──────────────────────────────────────────┐
     |                                                      |
     v                                                      v
[Process Supervisor]                                 [Hub State]
  - spawns child processes                               ^    ^
  - stdin/stdout/stderr pipes                           /      \
  - emits StateEvents ──────────────────────────────→ /        \
     |           |                                    /          \
     v           v                                   /            \
[MCP Client] [Log Aggregator]                       /              \
  - calls       - captures stderr              [MCP Client] [Supervisor]
    init,         - ring buffer                  writes         writes
    tools/list,   - broadcast channel            capabilities   process state
    ping          |
     |            v
     |       [Web UI / SSE]  ←── reads Hub State
     |            |               streams log broadcast
     v            v
  [Hub State]  HTTP :8080
  (capabilities,
   health)
                  |
            [Control Socket]  ←── CLI client (daemon mode)
                  |
            [Config Generator]  ←── reads Hub State or HubConfig
```

**Key data flows:**

1. **Startup:** Config Loader → Supervisor (spawn all servers) → MCP Client (initialize + introspect) → Hub State (populate capabilities)
2. **Crash recovery:** OS exit signal → Supervisor (detect exit, transition to BACKOFF) → Supervisor (wait backoff, re-spawn) → Hub State (update state)
3. **Log streaming:** Child stderr → Log Aggregator ring buffer → broadcast channel → Web UI SSE handler → browser
4. **MCP health check:** Timer tick → MCP Client (send ping on stdin) → read response from stdout → Hub State (update health) → Web UI status page
5. **CLI status:** CLI → Control Socket (if daemon) → serialize Hub State → print table

---

## Build Order (Dependency Graph)

Build components in this order. Each phase produces a working, testable artifact.

### Phase 1: Core (no async, no processes)
1. **`types.ts` equivalent: `types.rs`** — define all shared structs (`ServerConfig`, `HubConfig`, `ProcessState`, `LogLine`, `McpCapabilities`, `HealthStatus`, `ServerEntry`, `HubState`)
2. **Config Loader** — parse TOML into `HubConfig`, validate required fields, write unit tests with fixture TOML files

Deliverable: `mcp-hub --validate-config` works and rejects bad configs.

### Phase 2: Process management
3. **Process Supervisor** — spawn/stop/restart with tokio::process, state machine, backoff, emit state events
4. **Log Aggregator** — capture stderr, ring buffer, broadcast channel
5. **Hub State** — in-memory state aggregated from supervisor events

Deliverable: `mcp-hub start` spawns all configured servers, `mcp-hub status` prints states, Ctrl+C kills all cleanly.

### Phase 3: MCP protocol
6. **MCP Client** — JSON-RPC over stdio, initialize + tools/list + ping
7. Wire MCP Client into startup flow: after spawn, run initialize + introspection, write capabilities to Hub State
8. Wire health check timer: periodic ping, update Hub State health

Deliverable: `mcp-hub status` shows tool counts and health status per server.

### Phase 4: Web UI and log viewer
9. **Web UI Server** — Axum routes, status page, tools browser
10. **SSE log stream** — subscribe to log aggregator broadcast, forward to HTTP clients

Deliverable: `mcp-hub start --ui` serves a browser UI at localhost:8080.

### Phase 5: Config generator and daemon mode
11. **Config Generator** — Claude Code + Cursor snippet output, both offline and live modes
12. **Control Socket** — Unix socket IPC for daemon ↔ CLI communication
13. **Daemon mode** — fork/daemonize, PID file, `mcp-hub stop` via socket
14. **`mcp-hub init` wizard** — interactive TOML editor

Deliverable: full feature set, ready for distribution.

### Phase 6: Distribution
15. **Cross-compilation CI** — GitHub Actions matrix for macOS arm64/x86_64, Linux x86_64/aarch64, Windows
16. **Homebrew formula** — `brew install unityinflow/tap/mcp-hub`

---

## Key Architectural Tensions

### Stdout ownership: MCP channel vs. log capture
MCP servers use stdout as the JSON-RPC transport. The hub cannot treat stdout as a log stream. The supervisor must expose two separate channels per process: stdout (owned by the MCP client) and stderr (owned by the log aggregator). This is the most non-obvious constraint in the design.

### Foreground vs. daemon mode
In foreground mode, the supervisor, web UI, and log aggregator all run in the same Tokio runtime in the same process. In daemon mode, the supervisor daemon forks into the background and the CLI becomes a thin client connecting over a Unix socket. These two topologies share all the core components but differ only in the CLI layer and the control socket component. Design components to be runtime-agnostic — the supervisor should not know whether it is foreground or daemonized.

### Health check granularity
MCP `ping` is cheap but not always supported by all MCP server implementations. The health check should degrade gracefully: try MCP ping first; fall back to checking that the process is alive (PID check); report `degraded` vs. `unreachable` accordingly.

### Hub State contention
The web UI reads Hub State on every HTTP request. The supervisor and MCP client write to it on every state change. Use `tokio::sync::RwLock` (not `std::sync::RwLock`) to avoid blocking the async runtime. For the web UI status page, consider snapshotting state into a plain struct on each render rather than holding the lock across template rendering.

---

## Crate Structure

```
mcp-hub/
├── src/
│   ├── main.rs             <- entry point, Tokio runtime, CLI dispatch
│   ├── cli.rs              <- clap definitions, subcommand dispatch
│   ├── config.rs           <- TOML config types + loader
│   ├── types.rs            <- all shared domain types
│   ├── state.rs            <- HubState, Arc<RwLock<HubState>>
│   ├── supervisor.rs       <- process lifecycle, state machine, backoff
│   ├── logs.rs             <- log aggregator, ring buffer, broadcast
│   ├── mcp/
│   │   ├── mod.rs          <- McpClient, JSON-RPC helpers
│   │   ├── protocol.rs     <- request/response types (serde)
│   │   └── health.rs       <- periodic health check loop
│   ├── web/
│   │   ├── mod.rs          <- Axum router setup
│   │   ├── routes.rs       <- GET / , GET /servers/:name/tools
│   │   ├── sse.rs          <- SSE log stream handler
│   │   └── templates.rs    <- server-rendered HTML (string templates or askama)
│   ├── codegen.rs          <- config generator (Claude Code + Cursor)
│   └── control.rs          <- Unix socket IPC (daemon mode only)
├── tests/
│   ├── fixtures/
│   │   └── valid-config.toml
│   ├── config_test.rs
│   ├── supervisor_test.rs
│   └── mcp_client_test.rs
├── Cargo.toml
└── ...
```

---

*Research complete: 2026-04-02*
