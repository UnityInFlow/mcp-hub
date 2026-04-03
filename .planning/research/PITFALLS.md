# PITFALLS.md — mcp-hub

Critical mistakes to avoid when building a Rust-based MCP server process manager. Each pitfall is specific to the intersection of Tokio async, child process management, MCP protocol, and daemon architecture.

---

## 1. Zombie Processes from Dropped Child Handles

**What goes wrong:** `tokio::process::Child` must be explicitly awaited or have `kill_on_drop(true)` set. If the `Child` handle is dropped without waiting, the OS process becomes a zombie (on Unix) or continues running detached (on Windows). This is the most common silent failure in Tokio-based process managers.

**Warning signs:**
- `ps aux` shows orphaned processes after `mcp-hub stop`
- Process count climbs on repeated start/stop cycles
- Port conflicts on restart because the old process still holds the socket

**Prevention:**
- Always call `.wait()` or `.kill().await` before dropping a `Child`
- Use `kill_on_drop(true)` on `Command` only as a safety net, not the primary mechanism — it does not guarantee clean shutdown on all platforms
- Maintain a `JoinHandle` or `Arc<Mutex<Child>>` per managed server and drive shutdown explicitly through it
- Add an integration test that starts and stops a server 10 times in a loop and asserts process count stays constant

**Phase:** Week 13 (process lifecycle foundation)

---

## 2. stdin/stdout Pipes Blocking the Tokio Runtime

**What goes wrong:** MCP servers communicate over stdio. If you pipe stdin/stdout to the child (`Stdio::piped()`) but never consume the output, the child blocks when its pipe buffer fills (~64KB on Linux). This blocks the child process, which looks like a hang or health check failure — not a pipe issue.

**Warning signs:**
- Server appears healthy (process alive) but stops responding to MCP requests after bursts of output
- Health checks time out intermittently under load
- `strace` shows the child blocked on a `write()` syscall

**Prevention:**
- Always drain stdout/stderr with a dedicated `tokio::io::BufReader` task per child process
- Do not read stdout/stderr only when needed — drain them continuously into the log ring buffer regardless of whether anyone is watching
- When log streaming is inactive, still drain into a bounded circular buffer (e.g. last 1000 lines) so backpressure never builds
- Use `tokio::io::AsyncBufReadExt::lines()` for line-by-line draining

**Phase:** Week 13 (log aggregation design)

---

## 3. Exponential Backoff Without a Ceiling or Jitter

**What goes wrong:** Textbook exponential backoff (1s, 2s, 4s, 8s...) with a hard maximum is fine in isolation, but when 5+ servers all crash simultaneously (e.g. shared config mistake), they all restart in lock-step. They hammer any shared resource (port, file, external service) at the same cadence and all fail again together.

**Warning signs:**
- All servers cycle through their backoff in sync
- Restart storms: CPU spikes at exact intervals
- A broken server that depends on another server keeps resetting the backoff timer instead of making progress

**Prevention:**
- Add ±30% jitter to every backoff interval: `delay = base * 2^attempt * rand(0.7, 1.3)`
- Cap at 60s as specified, but also cap the attempt counter to prevent integer overflow on very long-running failures
- Track consecutive failures vs. total failures separately — a server that ran for 10 minutes and then crashed is healthier than one that crashed 3 times in 5 seconds; reset the backoff counter after a configurable minimum uptime (e.g. 30s)
- Implement a "give-up" threshold: after N attempts (e.g. 10), mark the server as `Failed` and stop restarting — require manual `mcp-hub restart <name>` to clear

**Phase:** Week 13 (health monitoring)

---

## 4. Conflating MCP Healthiness with Process Liveness

**What goes wrong:** Checking whether the process is alive (`child.try_wait()` returns `None`) is not the same as checking whether the MCP server is ready to serve requests. A process can be running but deadlocked, or still in initialization, or holding a mutex waiting for a resource. Reporting it as `healthy` in the web UI misleads users.

**Warning signs:**
- `mcp-hub status` shows all servers as running, but tool calls fail
- `mcp-hub init` generates config that includes servers that are not yet ready
- The config generator's `--live` mode returns empty tool lists because it introspects too early

**Prevention:**
- Define three distinct states: `Starting`, `Running` (process alive, no MCP confirmation), `Healthy` (MCP ping responded), `Degraded` (recent ping failures), `Failed`, `Stopped`
- Only mark a server `Healthy` after a successful `initialize` + `ping` (or `tools/list` as a proxy) round-trip
- Health checks should use a timeout (e.g. 5s) and treat non-response as `Degraded`, not `Failed` — degrade first, then fail after N consecutive misses
- The config generator `--live` mode must wait until servers reach `Healthy` state before introspecting

**Phase:** Week 13 (health check design) and Week 14 (web UI state display)

---

## 5. Signal Handling Races on Ctrl+C in Foreground Mode

**What goes wrong:** In foreground mode, Ctrl+C sends `SIGINT` to the entire process group. If child processes are in the same group (default for `tokio::process::Command`), they receive SIGINT directly and may exit before mcp-hub can drain their final log lines or update their status. This causes log truncation and incorrect final state.

**Warning signs:**
- Last few log lines from servers are missing after Ctrl+C
- Status file shows servers as `Running` after a clean shutdown
- Tests that send SIGINT to the manager process fail non-deterministically

**Prevention:**
- Spawn child processes in a new process group: `command.process_group(0)` (Unix) so they do not receive Ctrl+C directly
- Install a `tokio::signal::ctrl_c()` handler and perform ordered shutdown: send SIGTERM to all children, wait up to 5s, then SIGKILL any survivors
- Use a `tokio::sync::broadcast` shutdown channel so all log-draining tasks can finish flushing before the process exits
- On Windows, job objects are the equivalent mechanism — assign children to a job object so they are terminated when the parent exits

**Phase:** Week 13 (process spawning) — get this right before adding any other features on top

---

## 6. Blocking the Tokio Runtime with Synchronous I/O

**What goes wrong:** Reading the TOML config file, writing status to disk, or reading log history with `std::fs` inside an async task blocks the Tokio thread pool thread for the duration of the syscall. Under load, this starves other tasks. The symptom is subtle: health check timeouts that correlate with log writes, not with server load.

**Warning signs:**
- `tokio-console` shows tasks blocked in `std::fs` calls
- Health check latency spikes when log files are large
- `mcp-hub logs --follow` appears to lag when many servers are noisy

**Prevention:**
- Use `tokio::fs` for all file I/O in async contexts
- TOML config loading at startup is fine as synchronous (before the runtime starts), but status file writes during operation must be async
- For log ring buffers, prefer in-memory `VecDeque` with periodic async flushes rather than synchronous writes on every line
- Mark compute-heavy operations (e.g. regex matching on log output) with `tokio::task::spawn_blocking`

**Phase:** Week 13 (applies to all async code)

---

## 7. MCP JSON-RPC ID Correlation Across Concurrent Requests

**What goes wrong:** MCP is JSON-RPC 2.0. If you send `initialize`, `tools/list`, and `resources/list` concurrently to the same server (for faster introspection), you must match responses to requests by `id`. A naive implementation that reads the next response and assumes it matches the last request will silently return wrong data or deadlock waiting for a response that arrived out of order.

**Warning signs:**
- `tools/list` returns the response intended for `resources/list`
- Introspection hangs waiting for a response that was already consumed by a concurrent request
- Under fast config generation (`--live`), tool lists are occasionally empty or truncated

**Prevention:**
- Implement a proper JSON-RPC dispatcher: maintain a `HashMap<RequestId, oneshot::Sender<Response>>` per server connection
- Assign monotonically increasing request IDs per connection
- Run a single reader task per server that routes incoming messages to the correct waiting caller via the oneshot channel
- Do not assume request-response ordering even if the MCP server currently happens to maintain it — the protocol does not guarantee it

**Phase:** Week 14 (MCP introspection)

---

## 8. stdio vs. HTTP Transport Confusion

**What goes wrong:** MCP servers can use either stdio (most common for local tools) or HTTP/SSE transport. mcp-hub's v0.0.1 target is stdio-based servers, but the health check design and introspection code must not assume stdio everywhere. If transport type is not explicit in the TOML config, the manager may try to write JSON-RPC to stdin of an HTTP server that does not read stdin, causing silent failures.

**Warning signs:**
- HTTP-based MCP servers start successfully but health checks always fail
- `mcp-hub status` shows HTTP servers as `Degraded` immediately
- Config generator produces incomplete tool lists for HTTP servers

**Prevention:**
- Make `transport` an explicit required field in TOML config: `transport = "stdio"` or `transport = "http"`
- Fail fast with a clear error if transport is unrecognized rather than falling back silently
- Design the introspection client as a trait (`McpTransport`) with `StdioTransport` and `HttpTransport` implementations — even if only `StdioTransport` ships in v0.0.1, the abstraction prevents a rewrite later
- Document that v0.0.1 only supports stdio transport in the README

**Phase:** Week 13 (TOML config design) — the shape of this struct affects everything downstream

---

## 9. Daemon Mode IPC: PID Files Are Not Enough

**What goes wrong:** A simple PID file approach for daemon mode (`--daemon`) breaks in several real scenarios: stale PID file after a crash (the PID may be reused by another process), no way to check if the daemon is actually the mcp-hub process vs. an unrelated process with that PID, and no bidirectional communication for `status` and `stop` commands.

**Warning signs:**
- `mcp-hub stop` after a crash reports "daemon not running" or kills an unrelated process
- Two daemon instances start because both read a stale PID file and see "no process"
- `mcp-hub status` cannot get live data from the daemon — only what was last written to a state file

**Prevention:**
- Use a Unix domain socket (or named pipe on Windows) for daemon IPC, not a PID file alone
- Write both a PID file and a socket path; use the socket as the liveness check — if the socket is connectable, the daemon is alive
- On startup, attempt to connect to the socket first; if successful, refuse to start a second instance
- Write a simple request/response protocol over the socket for `status`, `stop`, `logs` commands — a newline-delimited JSON protocol reusing the same serde types as the web API works well
- On crash recovery, clean up stale socket files on startup after verifying the PID is dead

**Phase:** Week 13 (daemon mode design) — decide the IPC mechanism before building any commands that need it

---

## 10. Cross-Platform Process Management Assumptions

**What goes wrong:** Tokio's `Child::kill()` sends `SIGKILL` on Unix but calls `TerminateProcess` on Windows. Many Unix-specific patterns (SIGTERM for graceful shutdown, process groups, `/proc/<pid>/status`) simply do not exist on Windows. Building the Unix path first and retrofitting Windows later typically requires rewriting the shutdown and health check logic.

**Warning signs:**
- CI passes on Linux but Windows binary silently leaves processes running after stop
- `process_group(0)` compiles on Linux but panics or is a no-op on Windows
- Health check reads `/proc/<pid>/fd` to detect file descriptor leaks — Windows path not covered

**Prevention:**
- Audit every `unsafe` or platform-specific call at the design stage, not after the fact
- Use `#[cfg(unix)]` / `#[cfg(windows)]` blocks from day one for platform-diverging code, with compile-time errors for unimplemented platforms rather than silent wrong behavior
- Limit v0.0.1 to "works on macOS and Linux, Windows best-effort" if resource-constrained — but make this explicit in docs rather than discovering it in user bug reports
- Test the Windows path in CI from the first week — do not defer to release time

**Phase:** Week 13 (applies to all process management code), Week 15 (cross-compilation CI)

---

## 11. Web UI Serving Stale State

**What goes wrong:** The Axum web UI reads process state at request time. If state is stored in a `Mutex<HashMap<...>>` and the health check loop holds that mutex for a full introspection round (which can take hundreds of milliseconds for slow MCP servers), every web UI request queues behind it. Users see a UI that appears frozen or slow.

**Warning signs:**
- Web UI status page takes 200ms+ to load
- The status page and the health check loop have contention visible in `tokio-console`
- UI shows stale "last seen" timestamps when the health loop is running

**Prevention:**
- Separate the "health check write path" from the "web read path" using `tokio::sync::RwLock` — health checks get a write lock briefly to update state, web reads get a read lock
- Better: use `tokio::sync::watch` channels per server — health check loop sends state updates, web handler reads the latest value without blocking
- Store snapshots rather than live state in the web layer: the health loop pushes `ServerSnapshot` structs; the web handler just reads the latest snapshot
- Cap introspection (tools/list etc.) at 2s timeout so a slow server does not hold up the entire status update cycle

**Phase:** Week 14 (web UI + state management design)

---

## 12. Config Generator Producing Stale or Incorrect Tool Lists

**What goes wrong:** The Claude Code and Cursor config generators are one of the highest-value features. If they output incorrect tool names (because introspection ran before the server was fully initialized, or used a cached list from a previous run), users paste bad config into their AI tools and get silent failures. This is a trust-destroying bug.

**Warning signs:**
- Tool names in generated config do not match what `mcp-hub status` shows
- `--live` mode generates config identical to the TOML-only mode when servers are running
- Generated config works once but becomes stale after a server update

**Prevention:**
- `--live` mode must: (1) ensure all servers are in `Healthy` state, (2) call `tools/list`, `resources/list`, and `prompts/list` fresh (never cached), (3) fail loudly if any server is unreachable rather than silently omitting it
- Add a `--verify` flag that runs the generated config through a validation pass (check tool names are non-empty strings, no duplicates, valid JSON shape)
- Include a `generated_at` timestamp and `mcp_hub_version` in the output comment block so users know when the config was last regenerated
- Write an integration test that starts a known MCP server fixture, generates config, and asserts the tool list matches what the server advertises

**Phase:** Week 14 (config generator implementation)

---

## Summary — Phase Mapping

| Phase | Pitfalls to address |
|-------|---------------------|
| Week 13 — Process core | 1 (zombie processes), 2 (pipe blocking), 3 (backoff jitter), 4 (health state model), 5 (signal handling), 6 (blocking I/O), 8 (transport config shape), 9 (daemon IPC design), 10 (cross-platform) |
| Week 14 — Web UI + introspection | 4 (state display), 7 (JSON-RPC correlation), 11 (stale UI state), 12 (config generator accuracy) |
| Week 15 — Release + distribution | 10 (Windows CI validation) |
