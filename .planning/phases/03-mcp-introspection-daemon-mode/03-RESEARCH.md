# Phase 3: MCP Introspection & Daemon Mode — Research

**Written:** 2026-04-04
**Phase:** 03-mcp-introspection-daemon-mode
**Audience:** Planner who will produce 03-PLAN.md

---

## 1. MCP JSON-RPC Message Shapes

MCP over stdio is JSON-RPC 2.0. Every request is a single JSON object followed by `\n`. Every response is a single JSON object followed by `\n`. The existing `ping_server` in `health.rs` already proves this pattern works.

### 1.1 `initialize` — Handshake (must be first)

Request:
```json
{
  "jsonrpc": "2.0",
  "method": "initialize",
  "id": 1,
  "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {
      "name": "mcp-hub",
      "version": "0.0.1"
    }
  }
}
```

Response (success):
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {},
      "resources": {},
      "prompts": {}
    },
    "serverInfo": {
      "name": "my-server",
      "version": "1.0.0"
    }
  }
}
```

The `capabilities` map in the response tells which capability families the server supports. A server that omits `"tools"` from capabilities does not support `tools/list`. This should be respected: skip `tools/list` if `capabilities.tools` is absent.

After sending `initialize`, the server expects an `notifications/initialized` notification before it processes any further requests. This is a fire-and-forget JSON object with no `id`:
```json
{
  "jsonrpc": "2.0",
  "method": "notifications/initialized"
}
```
**Planning implication:** The introspection sequence is: send `initialize` → wait for response → send `notifications/initialized` (no wait) → then send the list requests concurrently.

### 1.2 `tools/list`

Request:
```json
{
  "jsonrpc": "2.0",
  "method": "tools/list",
  "id": 2
}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "read_file",
        "description": "Read a file from disk",
        "inputSchema": {
          "type": "object",
          "properties": {
            "path": { "type": "string" }
          },
          "required": ["path"]
        }
      }
    ]
  }
}
```

### 1.3 `resources/list`

Request:
```json
{
  "jsonrpc": "2.0",
  "method": "resources/list",
  "id": 3
}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "resources": [
      {
        "uri": "file:///some/path",
        "name": "project-file",
        "description": "A project file",
        "mimeType": "text/plain"
      }
    ]
  }
}
```

### 1.4 `prompts/list`

Request:
```json
{
  "jsonrpc": "2.0",
  "method": "prompts/list",
  "id": 4
}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "prompts": [
      {
        "name": "code-review",
        "description": "Review code for issues",
        "arguments": [
          { "name": "code", "description": "The code to review", "required": true }
        ]
      }
    ]
  }
}
```

### 1.5 Serde types to add to `protocol.rs`

The existing `JsonRpcResponse` parses `result` as `serde_json::Value`. For introspection, typed extraction works best as a second step: parse to `JsonRpcResponse` first (existing), then deserialize `result` into a typed struct using `serde_json::from_value`. This avoids changing the existing generic response type.

New types needed:
```rust
pub struct InitializeParams { pub protocol_version: String, pub capabilities: serde_json::Value, pub client_info: ClientInfo }
pub struct ClientInfo { pub name: String, pub version: String }
pub struct InitializeResult { pub protocol_version: String, pub capabilities: ServerCapabilities, pub server_info: Option<ServerInfo> }
pub struct ServerCapabilities { pub tools: Option<serde_json::Value>, pub resources: Option<serde_json::Value>, pub prompts: Option<serde_json::Value> }
pub struct ToolsListResult { pub tools: Vec<McpTool> }
pub struct McpTool { pub name: String, pub description: Option<String>, pub input_schema: Option<serde_json::Value> }
pub struct ResourcesListResult { pub resources: Vec<McpResource> }
pub struct McpResource { pub uri: String, pub name: String, pub description: Option<String>, pub mime_type: Option<String> }
pub struct PromptsListResult { pub prompts: Vec<McpPrompt> }
pub struct McpPrompt { pub name: String, pub description: Option<String>, pub arguments: Option<Vec<PromptArgument>> }
pub struct PromptArgument { pub name: String, pub description: Option<String>, pub required: Option<bool> }
```

All fields with `Option` because MCP servers vary in what they return.

---

## 2. Concurrent JSON-RPC ID Correlation — The Dispatcher Pattern

### The Problem

`run_health_check_loop` currently owns `stdin` and `stdout` exclusively. The health loop reads `stdout` in a loop. If introspection also reads `stdout`, both will race to consume responses, silently swallowing messages (Pitfall #7).

### The Solution: Single Reader Task + Dispatcher

Replace the per-ping read-in-place approach with a persistent reader task that owns stdout and routes every incoming line to a waiting caller via `oneshot` channels.

**Dispatcher state per server:**
```rust
type RequestId = u64;
type PendingMap = Arc<Mutex<HashMap<RequestId, oneshot::Sender<JsonRpcResponse>>>>;
```

**Architecture:**
1. One `reader_task` per server, spawned alongside the server process. It owns `BufReader<ChildStdout>` and loops calling `next_line().await`.
2. For each line, it parses `JsonRpcResponse`. It then looks up `id` in `pending_map`. If found, it sends the response through the `oneshot::Sender` and removes the entry. If not found (notification, unknown message), it logs and discards.
3. Any caller wanting to send a request: allocate an ID (atomic counter), create `oneshot::channel()`, insert `Sender` into `pending_map`, write the request JSON to stdin (via shared `Mutex<ChildStdin>`), then `.await` the `oneshot::Receiver`.

**Key detail — stdin sharing:** With a dispatcher, both health pings and introspection requests write to the same `ChildStdin`. Stdin must be wrapped in `Arc<Mutex<ChildStdin>>` so concurrent writes serialize correctly. Each writer lock→write→flush→unlock is fast (just bytes), so mutex contention is negligible.

**Concurrency for introspection:** After sending `initialize` and receiving its response (via oneshot), send `tools/list`, `resources/list`, and `prompts/list` *concurrently* by:
1. Registering three oneshot senders in pending_map.
2. Writing all three request lines to stdin in rapid succession (taking the stdin lock once per write).
3. Using `tokio::join!` to await all three receivers simultaneously.

The reader task naturally routes each response to the correct caller by ID, regardless of arrival order.

**Atomic ID allocation:**
```rust
use std::sync::atomic::{AtomicU64, Ordering};
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
fn next_id() -> u64 { REQUEST_ID.fetch_add(1, Ordering::Relaxed) }
```
Global is fine since IDs just need to be unique within a connection, and a monotonic global counter guarantees that. Alternatively, per-server AtomicU64 stored in a struct.

### Impact on `health.rs`

`run_health_check_loop` needs to be refactored. Instead of owning `ChildStdin` and `ChildStdout` directly, it receives:
- `stdin: Arc<Mutex<ChildStdin>>` (shared with introspection)
- `pending: PendingMap` (shared dispatcher map)

The existing `ping_server` function should be replaced or adapted to use the dispatcher: register a oneshot, write the ping, await the oneshot.

The current `ping_server` has a "drain up to 100 non-matching lines" loop specifically to handle out-of-order responses. With the dispatcher pattern, this drain loop disappears — the reader task handles routing, so the ping awaiter just waits for its specific ID.

---

## 3. Introspection + Health Check Coordination

### Current Architecture (Phase 2)

The health loop calls `run_health_check_loop` which takes ownership of `ChildStdin` and `ChildStdout`. There is no sharing mechanism.

### Phase 3 Architecture

The key change: **the reader task is the single owner of stdout**. Both health and introspection are writers-to-stdin + waiters-on-oneshot.

Flow at server startup:
1. Server spawned → `SpawnedProcess { stdin, stdout }` returned.
2. Before handing off to `run_server_supervisor`, create the shared structures:
   - `stdin_shared: Arc<Mutex<ChildStdin>>`
   - `pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>`
3. Spawn the `reader_task(stdout, pending.clone())` — this task runs for the server's lifetime.
4. Spawn `run_health_check_loop(stdin_shared.clone(), pending.clone(), ...)`.
5. When the server reaches `Running` state, trigger introspection: `run_introspection(stdin_shared.clone(), pending.clone(), snapshot_tx.clone())`.

Introspection runs once after first `Running` state transition. On restart, the `reader_task` and health loop are cancelled (via `health_cancel.cancel()`), the process is killed, a new process is spawned, and the same setup repeats.

**Timeout for introspection:** Each list request should have a 10s individual timeout (D-14 suggests 10s for operations that trigger introspection). The `tokio::time::timeout(Duration::from_secs(10), receiver.await)` pattern applies to each oneshot await.

---

## 4. Daemon Mode in Rust

### The `daemonize` crate (0.5.x) — already in STACK.md

The `daemonize` crate does the standard Unix double-fork:
1. First fork → parent exits, child continues (detaches from terminal).
2. `setsid()` → creates new session, child becomes session leader.
3. Second fork → prevents session leader from reacquiring a terminal.
4. Redirects stdin/stdout/stderr to `/dev/null`.
5. Writes PID file.

Usage pattern:
```rust
use daemonize::Daemonize;

fn daemonize_process(pid_file: &Path) -> anyhow::Result<()> {
    Daemonize::new()
        .pid_file(pid_file)
        .chown_pid_file(false)
        .working_directory("/")
        .start()
        .context("Failed to daemonize")?;
    Ok(())
}
```

**Critical timing issue:** Call `daemonize_process()` BEFORE creating the Tokio runtime. The `#[tokio::main]` macro creates the runtime immediately. Forking after Tokio runtime creation is unsafe because:
- Tokio spawns threads; `fork()` only copies the calling thread → other threads disappear in the child.
- Tokio's internal state (epoll fd, wakeup pipes) is not safe to share across fork.

**Solution:** In `main()`, parse CLI args, check if `--daemon`, then fork synchronously before `tokio::main` wraps everything. One way: use `tokio::main` but call a synchronous `maybe_daemonize()` function before the first `await`. Another cleaner way: use `#[tokio::main]` with a manual runtime builder:

```rust
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    
    if matches!(cli.command, Commands::Start { daemon: true, .. }) {
        daemonize_process(&pid_file_path)?;
        // After fork, we are in the daemon child process.
    }
    
    // Build Tokio runtime AFTER fork.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main(cli))
}
```

**Note:** `daemonize` is not in the current `Cargo.toml`. It must be added. Alternatively, use `nix::unistd::daemon(false, false)` which is already available via the `nix` crate (already in Cargo.toml). `nix::unistd::daemon(nochdir, noclose)` does the double-fork + setsid in one call. Using `nix` avoids adding a new dependency.

`nix::unistd::daemon(false, false)`:
- `nochdir = false` → changes to `/` (standard daemon behavior)
- `noclose = false` → closes and redirects stdin/stdout/stderr to `/dev/null`

PID file must then be written manually after the fork, since `nix::unistd::daemon` does not write a PID file. A manual write is straightforward:
```rust
std::fs::write(&pid_path, std::process::id().to_string())?;
```

### Socket Path and PID File Path (D-08: Claude's discretion)

Recommendation: `~/.config/mcp-hub/` (XDG compliant, matches `config_dir()` already used in `config.rs`):
- Socket: `~/.config/mcp-hub/mcp-hub.sock`
- PID: `~/.config/mcp-hub/mcp-hub.pid`

These paths should be constants resolved at runtime using `dirs::config_dir()`.

### Stale Socket Cleanup (D-07)

On startup with `--daemon`:
1. Try `UnixStream::connect(&socket_path)`. If succeeds → daemon is already running → print error and exit 1.
2. If connect fails:
   a. If socket file exists, check PID file. Read PID. Check if `kill(pid, 0)` succeeds (process alive). If PID is dead, remove socket and PID files.
   b. Proceed with daemonize.

---

## 5. Unix Socket IPC

### Server Side (inside daemon)

```rust
use tokio::net::UnixListener;

async fn run_control_socket(path: &Path, /* shared state */) -> anyhow::Result<()> {
    // Remove stale socket from previous run.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    
    loop {
        let (stream, _addr) = listener.accept().await?;
        // Spawn a task per connection so multiple CLI clients can connect simultaneously.
        tokio::spawn(handle_connection(stream, /* shared state */));
    }
}
```

### Client Side (CLI commands like `mcp-hub status`)

```rust
use tokio::net::UnixStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn send_daemon_command(socket_path: &Path, cmd: &DaemonRequest) -> anyhow::Result<DaemonResponse> {
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        UnixStream::connect(socket_path)
    ).await
    .context("Daemon connect timed out")?
    .context("Cannot connect to daemon socket")?;
    
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader).lines();
    
    let mut line = serde_json::to_string(cmd)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    
    let response_line = reader.next_line().await?
        .ok_or_else(|| anyhow::anyhow!("Daemon closed connection without responding"))?;
    
    let response: DaemonResponse = serde_json::from_str(&response_line)?;
    Ok(response)
}
```

### IPC Message Schema (D-05: Claude's discretion)

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    Status,
    Stop,
    Restart { name: String },
    Logs { server: Option<String>, lines: usize },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

Using `#[serde(tag = "cmd")]` means the JSON looks like `{"cmd": "status"}` or `{"cmd": "restart", "name": "my-server"}`. Clean and self-describing.

The daemon handler per connection: read one newline-delimited JSON line → parse `DaemonRequest` → execute → write one `DaemonResponse` line → close connection. Request-response, one round trip per connection. No long-lived streaming for Phase 3 (log follow streaming can be a follow-up — it requires the daemon to push lines continuously, which needs a different protocol mode).

### Concurrent CLI Connections

`tokio::spawn(handle_connection(...))` per accepted connection means multiple CLI clients (e.g. `mcp-hub status` and `mcp-hub restart foo` simultaneously) work correctly. The shared state (server handles, log aggregator, config) must be thread-safe behind `Arc`. Since we already use `Arc<LogAggregator>` and `watch::Receiver` per server, the existing design accommodates this.

---

## 6. SIGHUP Handler for Config Reload

### Tokio Signal Setup

```rust
#[cfg(unix)]
{
    use tokio::signal::unix::{signal, SignalKind};
    let mut sighup = signal(SignalKind::hangup())?;
    
    // Inside the main select! loop:
    _ = sighup.recv() => {
        handle_sighup_reload(&mut handles, &config_path, &shutdown, &log_agg).await;
    }
}
```

The `sighup.recv()` future is created once before the loop and polled in `tokio::select!`. It re-arms automatically — each call to `recv()` waits for the next SIGHUP.

### Config Diff Algorithm (D-09 through D-13)

`ServerConfig` currently derives `Debug, Clone, Serialize, Deserialize`. For diffing, add `PartialEq, Eq`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig { ... }
```

`HashMap` is `PartialEq` when values are `PartialEq`, so `HubConfig` can also derive it.

Note: `HashMap<String, String>` in `env` already derives `PartialEq`. The only complication is `f64` or `f32` fields — there are none in `ServerConfig`, so `Eq` is derivable.

Diff algorithm:
```rust
async fn apply_config_diff(
    handles: &mut Vec<ServerHandle>,
    old_config: &HubConfig,
    new_config: &HubConfig,
    shutdown: &CancellationToken,
    log_agg: &Arc<LogAggregator>,
) {
    let old_names: HashSet<_> = old_config.servers.keys().collect();
    let new_names: HashSet<_> = new_config.servers.keys().collect();
    
    // Removed servers: in old but not in new.
    for name in old_names.difference(&new_names) {
        stop_named_server(handles, name).await;
    }
    
    // New servers: in new but not in old.
    for name in new_names.difference(&old_names) {
        let cfg = &new_config.servers[*name];
        let handle = start_single_server(name, cfg, shutdown, log_agg).await;
        handles.push(handle);
    }
    
    // Changed servers: in both, but config differs.
    for name in old_names.intersection(&new_names) {
        let old_cfg = &old_config.servers[*name];
        let new_cfg = &new_config.servers[*name];
        if old_cfg != new_cfg {
            stop_named_server(handles, name).await;
            let handle = start_single_server(name, new_cfg, shutdown, log_agg).await;
            handles.push(handle);
        }
        // If equal: skip entirely (D-10).
    }
}
```

The `start_single_server` helper is a thin wrapper around the existing `run_server_supervisor` spawning logic, extracted from `start_all_servers`.

---

## 7. CLI Command Changes

### `Commands::Start` — add `--daemon` flag

```rust
/// Start all configured MCP servers.
Start {
    /// Run as a background daemon.
    #[arg(long)]
    daemon: bool,
}
```

When `daemon: true`:
1. Check if daemon is already running (attempt socket connect).
2. If running: print error, exit 1.
3. Daemonize (fork, setsid, redirect stdio).
4. Write PID file.
5. Continue with normal server startup + socket listener.

When `daemon: false`: existing `run_foreground_loop` behavior unchanged.

### `Commands::Stop`, `Restart`, `Status`, `Logs` — socket client

All four commands follow the same pattern:
1. Resolve socket path (`~/.config/mcp-hub/mcp-hub.sock`).
2. If socket not connectable: print "No daemon running. Use `mcp-hub start --daemon`" and exit 1 (D-15).
3. Send appropriate `DaemonRequest` variant.
4. Print `DaemonResponse` data to stdout.
5. Exit 0 on `ok: true`, exit 1 on `ok: false`.

**Timeouts (D-14):**
- `Status`: 5s (read-only query)
- `Stop`: 5s (command sent; shutdown is async in daemon)
- `Restart { name }`: 10s (may trigger introspection after restart)
- `Logs`: 5s (ring buffer read, no waiting)

---

## 8. `ServerSnapshot` and `McpCapabilities` Extension

Add to `types.rs`:
```rust
#[derive(Debug, Clone, Default)]
pub struct McpCapabilities {
    pub tools: Vec<McpTool>,
    pub resources: Vec<McpResource>,
    pub prompts: Vec<McpPrompt>,
    pub introspected_at: Option<std::time::Instant>,
}
```

Add `capabilities: McpCapabilities` field to `ServerSnapshot`.

The `status` command output can then show tool/resource/prompt counts alongside the existing state/health/PID/uptime columns.

---

## 9. File-by-File Change Summary

| File | Change |
|------|--------|
| `src/mcp/protocol.rs` | Add `InitializeRequest`, `NotificationsInitialized`, `ToolsListRequest`, `ResourcesListRequest`, `PromptsListRequest`, and all result types. |
| `src/mcp/health.rs` | Refactor `ping_server` and `run_health_check_loop` to use dispatcher (`Arc<Mutex<ChildStdin>>` + `PendingMap`) instead of owning stdio directly. |
| `src/mcp/introspect.rs` (new) | `run_introspection(stdin, pending, snapshot_tx)` — sends initialize + notifications/initialized + concurrent list requests, writes McpCapabilities to snapshot. |
| `src/mcp/dispatcher.rs` (new) | `reader_task(stdout, pending)` — the single-owner stdout reader that routes responses to pending oneshots. |
| `src/types.rs` | Add `McpCapabilities`, `McpTool`, `McpResource`, `McpPrompt` structs. Add `capabilities` field to `ServerSnapshot`. |
| `src/config.rs` | Add `PartialEq, Eq` derives to `ServerConfig` and `HubConfig`. |
| `src/supervisor.rs` | Wire dispatcher setup after spawn. Trigger introspection after `Running` state. Extract `start_single_server`. Add SIGHUP diff logic. |
| `src/control.rs` (new) | Unix socket listener, `DaemonRequest`/`DaemonResponse` types, `handle_connection`, `run_control_socket`. |
| `src/daemon.rs` (new) | `daemonize_process()`, `write_pid_file()`, `check_stale_socket()`, `resolve_socket_path()`, `resolve_pid_path()`. |
| `src/cli.rs` | Add `daemon: bool` to `Commands::Start`. |
| `src/main.rs` | Fork-before-runtime path for `--daemon`. Add SIGHUP handler in foreground loop. Replace stub exits with socket client calls. Start socket listener in daemon mode. |
| `Cargo.toml` | No new dependencies needed (using `nix` for daemonize, all other primitives in tokio). |

---

## 10. Critical Constraints and Risks

### Risk 1: Fork timing (daemonize before Tokio runtime)

Using `nix::unistd::daemon()` requires calling it synchronously before `tokio::runtime::Builder::build()`. The current `#[tokio::main]` macro creates the runtime in `main()` immediately. This means `main()` must be restructured: parse args synchronously, conditionally daemonize, then build the runtime manually.

### Risk 2: Reader task lifetime vs. process restart

When a server crashes and is restarted, the reader task is reading from the old stdout (now closed). It will get `None` from `next_line()` and must exit cleanly. The health cancel token must be cancelled before the process is killed so the reader task also exits. The `pending_map` must be drained (send errors to all waiting oneshotters) on reader task exit, otherwise introspection futures leak.

Drain on exit:
```rust
// In reader_task, after the read loop exits:
let mut pending = pending_map.lock().await;
for (_, sender) in pending.drain() {
    let _ = sender.send(/* error response or just drop */);
}
```

Dropping the `oneshot::Sender` without sending causes the receiver to get `Err(RecvError)`, which is what callers should handle.

### Risk 3: MCP servers that don't support all methods

Some MCP servers may not implement `resources/list` or `prompts/list`. The response will be a JSON-RPC error (`{"error": {...}}`). The introspection code must handle error responses gracefully — log a warning, store empty vec, continue. Do not fail the entire introspection because one list call returned an error.

### Risk 4: `notifications/initialized` requirement

Some strict MCP server implementations refuse to process list requests until they receive `notifications/initialized`. Forgetting this step causes `tools/list` to time out silently. This notification must be sent before the concurrent list requests.

### Risk 5: Windows compatibility

`nix::unistd::daemon()` is Unix-only. The daemon mode in Phase 3 should be `#[cfg(unix)]` only. On Windows, `mcp-hub start --daemon` should print "Daemon mode is not supported on Windows" and exit 1. The foreground mode works on all platforms.

---

## 11. Test Strategy for Phase 3

### Introspection tests

Use a minimal fake MCP server binary (a small Rust binary in `tests/fixtures/`) that:
- Reads JSON-RPC from stdin.
- Responds to `initialize`, `tools/list`, `resources/list`, `prompts/list`, `ping` with hardcoded responses.
- Can be configured (via env var or args) to return specific tool counts, delay responses, or return errors.

This fake server can be built as a `[[test-binary]]` in `Cargo.toml` or as a fixture script.

Test cases:
- Introspection captures correct tool/resource/prompt counts.
- Servers that return error on `resources/list` still populate tools and prompts.
- Out-of-order responses are correctly routed (send 3 concurrent requests, fake server responds in reverse order).
- Introspection times out cleanly if fake server delays > 10s.

### Daemon mode tests

- `mcp-hub start --daemon` creates socket file and PID file.
- Second `mcp-hub start --daemon` prints error and exits 1.
- `mcp-hub status` (daemon running) returns status table.
- `mcp-hub stop` (daemon running) sends stop command and daemon exits.
- After crash (kill -9 daemon), `mcp-hub start --daemon` cleans up stale files and starts fresh.

### Config reload tests

- SIGHUP with unchanged config: no restarts.
- SIGHUP adding a new server: new server starts.
- SIGHUP removing a server: server stops gracefully.
- SIGHUP changing a server's command: server restarts with new command.

---

## RESEARCH COMPLETE
