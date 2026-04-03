# Roadmap: mcp-hub

**Created:** 2026-04-02
**Phases:** 7
**Requirements:** 42 mapped

---

## Phase 1: Config & Process Supervisor

**Goal:** Parse and validate TOML config, then spawn/stop/restart child processes with correct signal handling and auto-restart backoff.

**Requirements:** CFG-01, CFG-02, PROC-01, PROC-02, PROC-03, PROC-05, PROC-06, PROC-07, PROC-08, PROC-09, DMN-01

### Success Criteria
1. `mcp-hub start` reads `mcp-hub.toml` and launches all configured servers; a typo in the config prints a clear error and exits non-zero.
2. `mcp-hub stop` sends SIGTERM to all children and waits up to 5 s before SIGKILL — no zombie processes remain.
3. `mcp-hub restart <name>` restarts only the named server while others keep running.
4. A server that exits immediately retries with exponential backoff (1 s → 2 s → 4 s…), is marked Fatal after N failures, and stops retrying.
5. Ctrl+C in foreground mode shuts down all children gracefully before the hub exits.

---

## Phase 2: Health Monitoring & Logging

**Goal:** Add per-server health state machine, periodic MCP ping checks, and a unified log aggregator with ring buffer and streaming.

**Requirements:** PROC-04, HLTH-01, HLTH-02, HLTH-03, HLTH-04, HLTH-05, LOG-01, LOG-02, LOG-03, LOG-04, LOG-05

### Success Criteria
1. `mcp-hub status` prints a table showing name, state (Starting / Running / Healthy / Degraded / Failed / Stopped), PID, uptime, and restart count.
2. `mcp-hub logs --follow` streams stderr from all servers interleaved, each line prefixed with timestamp and server name.
3. `mcp-hub logs --server <name> --follow` filters output to a single server.
4. A server that stops responding to MCP pings transitions through Degraded before reaching Failed — process liveness alone does not determine health.
5. A slow server's health check times out in ≤5 s and does not block health checks on other servers.

---

## Phase 3: MCP Introspection & Daemon Mode

**Goal:** Implement the JSON-RPC MCP client for capability discovery, and add background daemon mode with Unix socket IPC.

**Requirements:** MCP-01, MCP-02, MCP-03, MCP-04, CFG-03, DMN-02, DMN-03, DMN-04, DMN-05

### Success Criteria
1. On startup the hub sends `initialize` + `tools/list` + `resources/list` + `prompts/list` to each server and stores the results; concurrent requests use correctly correlated JSON-RPC IDs.
2. `mcp-hub status` reflects introspected tool/resource/prompt counts alongside process state.
3. `mcp-hub start --daemon` daemonizes the process; a second `mcp-hub start --daemon` call detects the running socket and exits with a clear error instead of starting a duplicate.
4. `mcp-hub stop` sent to a running daemon connects to the Unix socket and triggers graceful shutdown, confirmed by exit.
5. `kill -HUP <pid>` (or `mcp-hub reload`) reloads the TOML config without restarting already-running servers whose config is unchanged.

---

## Phase 4: Web UI

**Goal:** Serve an Axum-backed web interface with server status, tools browser, SSE log streaming, and a health endpoint for external monitoring.

**Requirements:** WEB-01, WEB-02, WEB-03, WEB-04, WEB-05

### Success Criteria
1. Opening `http://localhost:3456` in a browser shows all servers with name, state, PID, uptime, restart count, health state, and tool count.
2. The tools browser page lists each server's tools, resources, and prompts from the latest introspection.
3. The log viewer page streams live logs via SSE; selecting a server name filters the stream to that server only.
4. `GET /health` returns a JSON object with overall hub status and per-server health — responds in under 100 ms even with 10+ servers.
5. The UI port is configurable in `mcp-hub.toml` (default 3456); the hub starts even if no browser is open.

---

## Phase 5: Config Generation

**Goal:** Generate ready-to-paste Claude Code and Cursor config snippets from TOML config (offline) or live introspected state.

**Requirements:** GEN-01, GEN-02, GEN-03, GEN-04, GEN-05

### Success Criteria
1. `mcp-hub gen-config --format claude` outputs a valid `mcpServers` JSON block compatible with Claude Code `settings.json`, without any servers needing to be running.
2. `mcp-hub gen-config --format cursor` outputs a valid Cursor MCP config snippet.
3. `mcp-hub gen-config --format claude --live` connects to the running hub and includes the introspected tool/resource/prompt lists in the output.
4. Generated output includes a header comment with a timestamp and mcp-hub version string.
5. Running either gen-config subcommand on a config with zero servers exits with a clear warning rather than outputting empty JSON.

---

## Phase 6: Setup Wizard

**Goal:** Provide an interactive `mcp-hub init` wizard to add new servers to the TOML config without manual editing.

**Requirements:** WIZ-01, WIZ-02, WIZ-03

### Success Criteria
1. `mcp-hub init` launches an interactive prompt sequence asking for name, command, args, env vars, and transport type; pressing Enter skips optional fields with sensible defaults.
2. Running `mcp-hub init` in a directory with an existing `mcp-hub.toml` appends the new server entry without overwriting existing entries.
3. Running `mcp-hub init` in a directory with no config file creates a new `mcp-hub.toml` with the entered server as the first entry.
4. The wizard validates the server name is unique before writing; duplicate names produce an error prompt asking for a different name.

---

## Phase 7: Distribution

**Goal:** Produce pre-built binaries for all target platforms, publish to Homebrew tap, and enable `cargo install`.

**Requirements:** DIST-01, DIST-02, DIST-03

### Success Criteria
1. GitHub CI produces release artifacts for macOS arm64, macOS x86_64, Linux x86_64, Linux aarch64, and Windows x86_64 — all downloadable from the GitHub Releases page.
2. `brew install unityinflow/tap/mcp-hub` installs the binary on macOS with no Rust toolchain required.
3. `cargo install mcp-hub` succeeds on a clean machine with only a Rust stable toolchain installed.
4. All five platform binaries pass `mcp-hub --version` and `mcp-hub start` against a minimal fixture config in CI.

---

## Requirement Coverage

| Phase | Requirements | IDs |
|-------|-------------|-----|
| 1 | 11 | CFG-01, CFG-02, PROC-01, PROC-02, PROC-03, PROC-05, PROC-06, PROC-07, PROC-08, PROC-09, DMN-01 |
| 2 | 11 | PROC-04, HLTH-01, HLTH-02, HLTH-03, HLTH-04, HLTH-05, LOG-01, LOG-02, LOG-03, LOG-04, LOG-05 |
| 3 | 9 | MCP-01, MCP-02, MCP-03, MCP-04, CFG-03, DMN-02, DMN-03, DMN-04, DMN-05 |
| 4 | 5 | WEB-01, WEB-02, WEB-03, WEB-04, WEB-05 |
| 5 | 5 | GEN-01, GEN-02, GEN-03, GEN-04, GEN-05 |
| 6 | 3 | WIZ-01, WIZ-02, WIZ-03 |
| 7 | 3 | DIST-01, DIST-02, DIST-03 |
| **Total** | **47** | **42 v1 requirements + 5 adjusted (CFG-03 moved to Phase 3, PROC-04 moved to Phase 2)** |

> Note: CFG-03 (config reload) moved from the suggested Phase 3 to Phase 3 (unchanged). PROC-04 (status command) moved from the suggested Phase 2 to Phase 2 (unchanged). All 42 v1 requirements are mapped exactly once.

---

*Last updated: 2026-04-02*
