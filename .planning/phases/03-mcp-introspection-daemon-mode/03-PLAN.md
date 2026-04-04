---
plan_id: "03-03"
title: "Daemon Mode + Socket IPC"
phase: 3
wave: 2
depends_on:
  - "03-01"
files_modified:
  - src/daemon.rs
  - src/control.rs
  - src/cli.rs
  - src/main.rs
  - Cargo.toml
requirements_addressed:
  - DMN-02
  - DMN-03
  - DMN-05
autonomous: true
---

# Plan 03-03: Daemon Mode + Socket IPC

<objective>
Add background daemon mode (`mcp-hub start --daemon`) using `nix::unistd::daemon()`. Build the Unix
domain socket listener for daemon IPC with DaemonRequest/DaemonResponse types. Implement PID file
management, duplicate daemon prevention via socket liveness check, and stale socket cleanup. Restructure
main() from #[tokio::main] to manual Runtime::build() so fork happens before the Tokio runtime.
</objective>

---

## Task 1: Add `--daemon` flag to CLI

<task id="03-03-01">
<read_first>
- src/cli.rs (Commands enum, current Start variant)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 7 — CLI command changes)
</read_first>

<action>
In `src/cli.rs`:

1. **Add `daemon` field to `Commands::Start`**:
   ```rust
   /// Start all configured MCP servers.
   Start {
       /// Run as a background daemon.
       #[arg(long)]
       daemon: bool,
   },
   ```

2. **Update all `Commands::Start` match arms** in main.rs to destructure the new field:
   ```rust
   Commands::Start { daemon } => { ... }
   ```
</action>

<acceptance_criteria>
- grep: `daemon: bool` in src/cli.rs
- grep: `#\[arg(long)\]` before `daemon` in src/cli.rs
- grep: `Commands::Start { daemon }` in src/main.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Create the daemon module (daemonize, PID file, socket paths)

<task id="03-03-02">
<read_first>
- src/config.rs (find_and_load_config — uses dirs::config_dir)
- Cargo.toml (nix crate features)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 4 — daemon mode, Section 10 — Risk 1 fork timing, Risk 5 Windows)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-05 through D-08 — daemon architecture)
</read_first>

<action>
Create `src/daemon.rs`:

1. **Socket and PID path resolution**:
   ```rust
   use std::path::PathBuf;

   /// Resolve the daemon socket path: `~/.config/mcp-hub/mcp-hub.sock`
   pub fn socket_path() -> anyhow::Result<PathBuf> {
       let config_dir = dirs::config_dir()
           .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
       let dir = config_dir.join("mcp-hub");
       std::fs::create_dir_all(&dir)
           .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", dir.display()))?;
       Ok(dir.join("mcp-hub.sock"))
   }

   /// Resolve the daemon PID file path: `~/.config/mcp-hub/mcp-hub.pid`
   pub fn pid_path() -> anyhow::Result<PathBuf> {
       let config_dir = dirs::config_dir()
           .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
       Ok(config_dir.join("mcp-hub").join("mcp-hub.pid"))
   }
   ```

2. **Write PID file**:
   ```rust
   pub fn write_pid_file(path: &std::path::Path) -> anyhow::Result<()> {
       std::fs::write(path, std::process::id().to_string())
           .map_err(|e| anyhow::anyhow!("Failed to write PID file {}: {e}", path.display()))
   }
   ```

3. **Remove PID file** (cleanup):
   ```rust
   pub fn remove_pid_file(path: &std::path::Path) {
       let _ = std::fs::remove_file(path);
   }
   ```

4. **Check if daemon is already running** (D-07 — socket liveness):
   ```rust
   #[cfg(unix)]
   pub fn check_existing_daemon(sock_path: &std::path::Path, pid_path: &std::path::Path) -> anyhow::Result<()> {
       use std::os::unix::net::UnixStream as StdUnixStream;

       // Try connecting to the socket synchronously (before Tokio runtime exists).
       match StdUnixStream::connect(sock_path) {
           Ok(_) => {
               // Socket is live — a daemon is running.
               anyhow::bail!(
                   "A daemon is already running (socket: {}). Use `mcp-hub stop` to stop it.",
                   sock_path.display()
               );
           }
           Err(_) => {
               // Socket not connectable. Check for stale files.
               cleanup_stale_files(sock_path, pid_path);
               Ok(())
           }
       }
   }
   ```

5. **Cleanup stale files** when PID is dead:
   ```rust
   #[cfg(unix)]
   fn cleanup_stale_files(sock_path: &std::path::Path, pid_path: &std::path::Path) {
       // Read PID file and check if process is alive.
       if let Ok(pid_str) = std::fs::read_to_string(pid_path) {
           if let Ok(pid) = pid_str.trim().parse::<i32>() {
               use nix::sys::signal::{kill, Signal};
               use nix::unistd::Pid;

               // kill(pid, 0) checks if process exists without sending a signal.
               if kill(Pid::from_raw(pid), Signal::SIGKILL).is_err() {
                   // Use Signal "0" check — actually use kill with None:
               }
               // Simplified: try kill(pid, 0). If ESRCH, process is dead.
               match nix::sys::signal::kill(Pid::from_raw(pid), None) {
                   Err(nix::errno::Errno::ESRCH) => {
                       // Process is dead — clean up stale files.
                       tracing::info!("Cleaning up stale daemon files (PID {pid} is dead)");
                       let _ = std::fs::remove_file(sock_path);
                       let _ = std::fs::remove_file(pid_path);
                   }
                   Ok(()) => {
                       // Process alive but socket dead — unusual. Clean up socket only.
                       tracing::warn!("PID {pid} alive but socket not connectable — removing stale socket");
                       let _ = std::fs::remove_file(sock_path);
                   }
                   Err(_) => {
                       // Permission error or other — clean up both to be safe.
                       let _ = std::fs::remove_file(sock_path);
                       let _ = std::fs::remove_file(pid_path);
                   }
               }
           }
       } else {
           // No PID file — just remove stale socket.
           if sock_path.exists() {
               let _ = std::fs::remove_file(sock_path);
           }
       }
   }
   ```

6. **Daemonize the process** (Unix only):
   ```rust
   #[cfg(unix)]
   pub fn daemonize_process() -> anyhow::Result<()> {
       nix::unistd::daemon(false, false)
           .map_err(|e| anyhow::anyhow!("Failed to daemonize: {e}"))?;
       Ok(())
   }

   #[cfg(not(unix))]
   pub fn daemonize_process() -> anyhow::Result<()> {
       anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.")
   }
   ```

7. **Add `mod daemon;`** to `src/main.rs`.
</action>

<acceptance_criteria>
- grep: `pub fn socket_path` in src/daemon.rs
- grep: `pub fn pid_path` in src/daemon.rs
- grep: `pub fn write_pid_file` in src/daemon.rs
- grep: `pub fn remove_pid_file` in src/daemon.rs
- grep: `pub fn check_existing_daemon` in src/daemon.rs
- grep: `pub fn daemonize_process` in src/daemon.rs
- grep: `nix::unistd::daemon` in src/daemon.rs
- grep: `fn cleanup_stale_files` in src/daemon.rs
- grep: `mcp-hub.sock` in src/daemon.rs
- grep: `mcp-hub.pid` in src/daemon.rs
- grep: `mod daemon` in src/main.rs
- No `unwrap()` in src/daemon.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Create the control socket module (server side + IPC types)

<task id="03-03-03">
<read_first>
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 5 — Unix socket IPC, DaemonRequest/DaemonResponse)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-05, D-14 — socket protocol, timeouts)
- src/supervisor.rs (ServerHandle, SupervisorCommand — state the daemon needs access to)
- src/logs.rs (LogAggregator — for logs command)
</read_first>

<action>
Create `src/control.rs`:

1. **IPC message types**:
   ```rust
   use serde::{Deserialize, Serialize};

   #[derive(Debug, Serialize, Deserialize)]
   #[serde(tag = "cmd", rename_all = "snake_case")]
   pub enum DaemonRequest {
       Status,
       Stop,
       Restart { name: String },
       Logs { server: Option<String>, lines: usize },
       Reload,
   }

   #[derive(Debug, Serialize, Deserialize)]
   pub struct DaemonResponse {
       pub ok: bool,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub data: Option<serde_json::Value>,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub error: Option<String>,
   }

   impl DaemonResponse {
       pub fn success(data: serde_json::Value) -> Self {
           Self {
               ok: true,
               data: Some(data),
               error: None,
           }
       }

       pub fn ok_empty() -> Self {
           Self {
               ok: true,
               data: None,
               error: None,
           }
       }

       pub fn err(message: String) -> Self {
           Self {
               ok: false,
               data: None,
               error: Some(message),
           }
       }
   }
   ```

2. **Socket listener (server side)** — runs inside the daemon:
   ```rust
   use std::path::Path;
   use std::sync::Arc;

   use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
   use tokio::net::UnixListener;
   use tokio_util::sync::CancellationToken;

   use crate::logs::LogAggregator;
   use crate::supervisor::ServerHandle;

   /// Shared state accessible to all connection handlers.
   pub struct DaemonState {
       pub handles: Arc<tokio::sync::Mutex<Vec<ServerHandle>>>,
       pub log_agg: Arc<LogAggregator>,
       pub shutdown: CancellationToken,
       pub color: bool,
   }

   pub async fn run_control_socket(
       sock_path: &Path,
       state: Arc<DaemonState>,
   ) -> anyhow::Result<()> {
       // Remove stale socket from previous run.
       let _ = std::fs::remove_file(sock_path);

       let listener = UnixListener::bind(sock_path)
           .map_err(|e| anyhow::anyhow!("Failed to bind socket {}: {e}", sock_path.display()))?;

       tracing::info!("Control socket listening on {}", sock_path.display());

       loop {
           tokio::select! {
               accept = listener.accept() => {
                   match accept {
                       Ok((stream, _addr)) => {
                           let state = Arc::clone(&state);
                           tokio::spawn(async move {
                               if let Err(e) = handle_connection(stream, state).await {
                                   tracing::warn!("Control socket connection error: {e}");
                               }
                           });
                       }
                       Err(e) => {
                           tracing::warn!("Failed to accept control socket connection: {e}");
                       }
                   }
               }
               _ = state.shutdown.cancelled() => {
                   tracing::debug!("Control socket shutting down");
                   break;
               }
           }
       }

       // Cleanup socket file on shutdown.
       let _ = std::fs::remove_file(sock_path);
       Ok(())
   }
   ```

3. **Connection handler** — one request-response per connection:
   ```rust
   async fn handle_connection(
       stream: tokio::net::UnixStream,
       state: Arc<DaemonState>,
   ) -> anyhow::Result<()> {
       let (reader, mut writer) = stream.into_split();
       let mut lines = BufReader::new(reader).lines();

       let request_line = lines.next_line().await?
           .ok_or_else(|| anyhow::anyhow!("Client disconnected without sending request"))?;

       let request: DaemonRequest = serde_json::from_str(&request_line)
           .map_err(|e| anyhow::anyhow!("Invalid request JSON: {e}"))?;

       let response = dispatch_request(request, &state).await;

       let mut response_json = serde_json::to_string(&response)?;
       response_json.push('\n');
       writer.write_all(response_json.as_bytes()).await?;
       writer.flush().await?;

       Ok(())
   }
   ```

4. **Request dispatch** — handle each DaemonRequest variant:
   ```rust
   async fn dispatch_request(
       request: DaemonRequest,
       state: &DaemonState,
   ) -> DaemonResponse {
       match request {
           DaemonRequest::Status => {
               let handles = state.handles.lock().await;
               let states: Vec<_> = handles.iter().map(|h| {
                   let snapshot = h.state_rx.borrow().clone();
                   serde_json::json!({
                       "name": h.name,
                       "state": snapshot.process_state.to_string(),
                       "health": snapshot.health.to_string(),
                       "pid": snapshot.pid,
                       "restart_count": snapshot.restart_count,
                       "transport": snapshot.transport,
                       "tools": snapshot.capabilities.tools.len(),
                       "resources": snapshot.capabilities.resources.len(),
                       "prompts": snapshot.capabilities.prompts.len(),
                   })
               }).collect();
               DaemonResponse::success(serde_json::json!(states))
           }

           DaemonRequest::Stop => {
               tracing::info!("Stop command received via control socket");
               state.shutdown.cancel();
               DaemonResponse::ok_empty()
           }

           DaemonRequest::Restart { name } => {
               let handles = state.handles.lock().await;
               match crate::supervisor::restart_server(&handles, &name).await {
                   Ok(()) => DaemonResponse::ok_empty(),
                   Err(e) => DaemonResponse::err(e.to_string()),
               }
           }

           DaemonRequest::Logs { server, lines } => {
               let log_lines = match &server {
                   Some(name) => {
                       match state.log_agg.get_buffer(name) {
                           Some(buf) => {
                               let all = buf.snapshot_last(lines).await;
                               all.iter()
                                   .map(|l| crate::logs::format_log_line(l, false))
                                   .collect::<Vec<_>>()
                           }
                           None => return DaemonResponse::err(format!("Unknown server: '{name}'")),
                       }
                   }
                   None => {
                       let all = state.log_agg.snapshot_all().await;
                       let tail = if all.len() > lines { &all[all.len() - lines..] } else { &all[..] };
                       tail.iter()
                           .map(|l| crate::logs::format_log_line(l, false))
                           .collect::<Vec<_>>()
                   }
               };
               DaemonResponse::success(serde_json::json!(log_lines))
           }

           DaemonRequest::Reload => {
               // Placeholder — wired in Plan 03-04.
               DaemonResponse::err("Reload not yet implemented".to_string())
           }
       }
   }
   ```

5. **Socket client** (CLI side) — for commands that connect to a running daemon:
   ```rust
   use std::time::Duration;

   pub async fn send_daemon_command(
       sock_path: &Path,
       request: &DaemonRequest,
       timeout_secs: u64,
   ) -> anyhow::Result<DaemonResponse> {
       use tokio::net::UnixStream;

       let stream = tokio::time::timeout(
           Duration::from_secs(timeout_secs),
           UnixStream::connect(sock_path),
       )
       .await
       .map_err(|_| anyhow::anyhow!("Connection to daemon timed out after {timeout_secs}s"))?
       .map_err(|e| anyhow::anyhow!(
           "Cannot connect to daemon socket ({}): {e}\nIs the daemon running? Start with: mcp-hub start --daemon",
           sock_path.display()
       ))?;

       let (reader, mut writer) = stream.into_split();
       let mut lines = BufReader::new(reader).lines();

       let mut json = serde_json::to_string(request)?;
       json.push('\n');
       writer.write_all(json.as_bytes()).await?;
       writer.flush().await?;

       let response_line = tokio::time::timeout(
           Duration::from_secs(timeout_secs),
           lines.next_line(),
       )
       .await
       .map_err(|_| anyhow::anyhow!("Daemon response timed out after {timeout_secs}s"))?
       .map_err(|e| anyhow::anyhow!("Error reading daemon response: {e}"))?
       .ok_or_else(|| anyhow::anyhow!("Daemon closed connection without responding"))?;

       let response: DaemonResponse = serde_json::from_str(&response_line)
           .map_err(|e| anyhow::anyhow!("Invalid daemon response JSON: {e}"))?;

       Ok(response)
   }
   ```

6. **Add `mod control;`** to `src/main.rs`.
</action>

<acceptance_criteria>
- grep: `pub enum DaemonRequest` in src/control.rs
- grep: `pub struct DaemonResponse` in src/control.rs
- grep: `pub struct DaemonState` in src/control.rs
- grep: `pub async fn run_control_socket` in src/control.rs
- grep: `async fn handle_connection` in src/control.rs
- grep: `async fn dispatch_request` in src/control.rs
- grep: `pub async fn send_daemon_command` in src/control.rs
- grep: `DaemonRequest::Status` in src/control.rs
- grep: `DaemonRequest::Stop` in src/control.rs
- grep: `DaemonRequest::Restart` in src/control.rs
- grep: `DaemonRequest::Logs` in src/control.rs
- grep: `DaemonRequest::Reload` in src/control.rs
- grep: `mod control` in src/main.rs
- grep: `serde(tag = "cmd"` in src/control.rs
- No `unwrap()` in src/control.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Restructure main() for fork-before-runtime and daemon startup

<task id="03-03-04">
<read_first>
- src/main.rs (current #[tokio::main] structure, Commands::Start, run_foreground_loop)
- src/daemon.rs (daemonize_process, check_existing_daemon, write_pid_file, socket_path, pid_path — from Task 2)
- src/control.rs (run_control_socket, DaemonState — from Task 3)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 4 — fork timing, manual Runtime::build)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-05 through D-07)
</read_first>

<action>
Restructure `src/main.rs`:

1. **Remove `#[tokio::main]`** and replace with a synchronous `main()`:
   ```rust
   fn main() -> anyhow::Result<()> {
       let cli = Cli::parse();

       // Daemonize BEFORE creating the Tokio runtime (Risk 1).
       let is_daemon = matches!(cli.command, Commands::Start { daemon: true });

       #[cfg(unix)]
       if is_daemon {
           let sock = daemon::socket_path()?;
           let pid = daemon::pid_path()?;
           daemon::check_existing_daemon(&sock, &pid)?;
           daemon::daemonize_process()?;
           daemon::write_pid_file(&pid)?;
       }

       #[cfg(not(unix))]
       if is_daemon {
           anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.");
       }

       // Build Tokio runtime AFTER fork.
       tokio::runtime::Builder::new_multi_thread()
           .enable_all()
           .build()?
           .block_on(async_main(cli))
   }
   ```

2. **Move the current `main()` body** into `async fn async_main(cli: Cli) -> anyhow::Result<()>`:
   - Keep all current logic intact.
   - The `Commands::Start { daemon }` branch differentiates behavior:
     - If `daemon == true`: start servers, start control socket listener, skip foreground loop (no stdin in daemon), wait for shutdown signal via the control socket stop command or SIGTERM.
     - If `daemon == false`: existing foreground behavior unchanged.

3. **Daemon mode start path**:
   ```rust
   Commands::Start { daemon } => {
       // ... config loading (same as before) ...

       let shutdown = CancellationToken::new();
       let log_agg = Arc::new(logs::LogAggregator::new(&server_names, 10_000));
       let mut handles = supervisor::start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

       supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

       if daemon {
           // Daemon mode — run control socket instead of foreground loop.
           let sock = daemon::socket_path()?;
           let pid = daemon::pid_path()?;

           let daemon_state = Arc::new(control::DaemonState {
               handles: Arc::new(tokio::sync::Mutex::new(handles)),
               log_agg: Arc::clone(&log_agg),
               shutdown: shutdown.clone(),
               color: false, // no TTY in daemon mode
           });

           // Run control socket listener.
           let socket_task = tokio::spawn({
               let state = Arc::clone(&daemon_state);
               async move {
                   if let Err(e) = control::run_control_socket(&sock, state).await {
                       tracing::error!("Control socket error: {e}");
                   }
               }
           });

           // Wait for shutdown (triggered by Stop command or SIGTERM).
           wait_for_shutdown_signal().await?;

           tracing::info!("Shutting down daemon...");
           shutdown.cancel();

           // Wait for control socket to close.
           socket_task.await.ok();

           // Stop all servers.
           let final_handles = Arc::try_unwrap(daemon_state.handles)
               .map_err(|_| anyhow::anyhow!("Cannot unwrap daemon handles"))?
               .into_inner();
           supervisor::stop_all_servers(final_handles).await;

           // Cleanup PID file.
           daemon::remove_pid_file(&pid);

           tracing::info!("Daemon stopped.");
       } else {
           // Foreground mode — existing behavior.
           let states = output::collect_states_from_handles(&handles);
           output::print_status_table(&states, color);

           run_foreground_loop(&handles, color, Arc::clone(&log_agg)).await?;

           tracing::info!("Shutting down all servers...");
           shutdown.cancel();
           supervisor::stop_all_servers(handles).await;
           tracing::info!("All servers stopped.");
       }

       Ok(())
   }
   ```

4. **Add `use crate::daemon` and `use crate::control`** to main.rs imports.

5. **Ensure `wait_for_shutdown_signal`** works for daemon mode (SIGTERM-only in daemon, no stdin).
</action>

<acceptance_criteria>
- grep: `fn main() -> anyhow::Result` in src/main.rs (synchronous main, not async)
- grep: `async fn async_main` in src/main.rs
- grep: `tokio::runtime::Builder::new_multi_thread` in src/main.rs
- grep: `daemon::daemonize_process` in src/main.rs
- grep: `daemon::check_existing_daemon` in src/main.rs
- grep: `daemon::write_pid_file` in src/main.rs
- grep: `daemon::remove_pid_file` in src/main.rs
- grep: `control::run_control_socket` in src/main.rs
- grep: `DaemonState` in src/main.rs
- No `#[tokio::main]` in src/main.rs
- cargo build succeeds
- cargo clippy -- -D warnings passes
</acceptance_criteria>
</task>

---

## Task 5: Daemon mode tests

<task id="03-03-05">
<read_first>
- src/daemon.rs (all functions — from Task 2)
- src/control.rs (DaemonRequest, DaemonResponse, send_daemon_command — from Task 3)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 11 — daemon mode tests)
</read_first>

<action>
Create `tests/daemon.rs`:

1. **Test: socket_path and pid_path return valid paths**:
   - Call both functions, assert they end with `mcp-hub.sock` and `mcp-hub.pid` respectively.
   - Assert the parent directory exists (or can be created).

2. **Test: write_pid_file and read back**:
   - Write PID file to a temp directory.
   - Read it back and parse as u32.
   - Assert it equals `std::process::id()`.

3. **Test: check_existing_daemon with no socket**:
   - Point to a non-existent socket path.
   - Assert `check_existing_daemon` returns Ok (no daemon running).

4. **Test: DaemonRequest serialization round-trip**:
   - Serialize each variant (Status, Stop, Restart, Logs, Reload) to JSON.
   - Deserialize back and verify equality.
   - Assert `{"cmd":"status"}` for Status.
   - Assert `{"cmd":"restart","name":"foo"}` for Restart { name: "foo" }.

5. **Test: DaemonResponse constructors**:
   - `DaemonResponse::success(json!("test"))` has ok=true, data=Some, error=None.
   - `DaemonResponse::err("fail".into())` has ok=false, data=None, error=Some.
   - `DaemonResponse::ok_empty()` has ok=true, data=None, error=None.

6. **Test: control socket round-trip** (integration):
   - Spawn a minimal control socket listener on a temp socket path with a mock DaemonState.
   - Use `send_daemon_command` to send a Status request.
   - Assert response has ok=true and data contains an array.
   - Clean up temp socket.

Note: Full daemon lifecycle tests (start --daemon, stop via socket, stale cleanup) are complex integration tests that will be covered in Plan 03-04's integration test task.
</action>

<acceptance_criteria>
- grep: `fn socket_path_returns_valid` in tests/daemon.rs
- grep: `fn write_and_read_pid_file` in tests/daemon.rs
- grep: `fn daemon_request_serialization` in tests/daemon.rs
- grep: `fn daemon_response_constructors` in tests/daemon.rs
- grep: `fn control_socket_round_trip` in tests/daemon.rs
- cargo test --test daemon passes
- cargo clippy -- -D warnings passes
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run in sequence:

```bash
cargo build 2>&1 | head -5
cargo clippy -- -D warnings 2>&1 | head -10
cargo test 2>&1
cargo fmt -- --check 2>&1 | head -5
```

### must_haves
- [ ] `--daemon` flag added to `Commands::Start`
- [ ] main() is synchronous, creates Tokio runtime manually after fork
- [ ] `nix::unistd::daemon(false, false)` called before runtime creation
- [ ] PID file written after fork, cleaned up on shutdown
- [ ] Socket path: `~/.config/mcp-hub/mcp-hub.sock`
- [ ] Duplicate daemon detected by socket connect attempt before daemonize
- [ ] Stale socket cleaned up when PID is dead
- [ ] DaemonRequest is serde-tagged enum with Status/Stop/Restart/Logs/Reload
- [ ] DaemonResponse has ok/data/error fields
- [ ] Control socket accepts connections, dispatches requests, sends responses
- [ ] One request-response per connection (newline-delimited JSON)
- [ ] Control socket task spawned per connection for concurrency
- [ ] Control socket shuts down when shutdown token is cancelled
- [ ] Daemon mode: no foreground loop (no stdin), waits for SIGTERM or Stop command
- [ ] Foreground mode: unchanged behavior from Phase 2
- [ ] Windows: `--daemon` prints error and exits 1
- [ ] All daemon unit tests pass
- [ ] All existing tests still pass
- [ ] No unwrap() in production code (src/)
- [ ] cargo clippy -D warnings passes
</verification>
