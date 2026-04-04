---
plan_id: "03-04"
title: "CLI Wiring + Config Reload + Integration Tests"
phase: 3
wave: 3
depends_on:
  - "03-02"
  - "03-03"
files_modified:
  - src/main.rs
  - src/config.rs
  - src/supervisor.rs
  - src/control.rs
  - src/cli.rs
  - tests/cli_daemon.rs
  - tests/config_reload.rs
requirements_addressed:
  - CFG-03
  - DMN-04
autonomous: true
---

# Plan 03-04: CLI Wiring + Config Reload + Integration Tests

<objective>
Wire all CLI subcommands (stop, restart, status, logs) to the daemon's Unix socket via the
send_daemon_command client. Implement SIGHUP-based config reload with PartialEq diff on
ServerConfig (add/remove/change/unchanged). Add a `reload` CLI command. Write integration
tests covering the full daemon lifecycle and config reload scenarios.
</objective>

---

## Task 1: Derive PartialEq + Eq on ServerConfig and HubConfig

<task id="03-04-01">
<read_first>
- src/config.rs (ServerConfig, HubConfig — current derives)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 6 — config diff, PartialEq derive)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-09 — PartialEq on ServerConfig)
</read_first>

<action>
In `src/config.rs`:

1. **Add `PartialEq, Eq` to `ServerConfig`**:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
   pub struct ServerConfig { ... }
   ```

2. **Add `PartialEq` to `HubConfig`**:
   ```rust
   #[derive(Debug, Serialize, Deserialize, PartialEq)]
   pub struct HubConfig { ... }
   ```
   Note: `HubConfig` contains `HashMap<String, ServerConfig>` which is `PartialEq` when values are `PartialEq`. `Eq` is also derivable since all fields are `Eq` (String, Vec<String>, HashMap<String, String>, Option<String>, Option<u64>, Option<u32>).

3. **Verify no `f32`/`f64` fields** that would prevent `Eq` derivation. Current `ServerConfig` has no float fields (restart_delay and health_check_interval are `Option<u64>`). Confirm this.
</action>

<acceptance_criteria>
- grep: `PartialEq, Eq` in src/config.rs (ServerConfig derive)
- grep: `PartialEq` in src/config.rs (HubConfig derive)
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 2: Implement config diff and apply logic

<task id="03-04-02">
<read_first>
- src/config.rs (ServerConfig with PartialEq — from Task 1, load_config, find_and_load_config)
- src/supervisor.rs (start_all_servers, stop_all_servers, restart_server, ServerHandle, run_server_supervisor)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 6 — full diff algorithm with HashSet)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-09 through D-13 — unchanged/changed/new/removed)
</read_first>

<action>
In `src/supervisor.rs`:

1. **Extract `start_single_server` helper** from the existing `start_all_servers` loop body:
   ```rust
   pub async fn start_single_server(
       name: &str,
       config: &ServerConfig,
       shutdown: &CancellationToken,
       log_agg: &Arc<LogAggregator>,
   ) -> ServerHandle {
       let initial_snapshot = ServerSnapshot {
           transport: config.transport.clone(),
           ..ServerSnapshot::default()
       };
       let (state_tx, state_rx) = tokio::sync::watch::channel(initial_snapshot);
       let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(8);
       let token = shutdown.child_token();
       let name_owned = name.to_string();
       let cfg = config.clone();
       let agg = Arc::clone(log_agg);

       let task = tokio::spawn(async move {
           run_server_supervisor(name_owned, cfg, token, cmd_rx, state_tx, agg).await;
       });

       ServerHandle {
           name: name.to_string(),
           state_rx,
           cmd_tx,
           task,
       }
   }
   ```

2. **Refactor `start_all_servers`** to use `start_single_server`:
   ```rust
   pub async fn start_all_servers(...) -> Vec<ServerHandle> {
       let mut handles = Vec::with_capacity(config.servers.len());
       for (name, server_config) in &config.servers {
           handles.push(start_single_server(name, server_config, &shutdown, &log_agg).await);
       }
       handles
   }
   ```

3. **Add `stop_named_server` helper**:
   ```rust
   pub async fn stop_named_server(handles: &mut Vec<ServerHandle>, name: &str) {
       if let Some(pos) = handles.iter().position(|h| h.name == name) {
           let handle = handles.remove(pos);
           let _ = handle.cmd_tx.send(SupervisorCommand::Shutdown).await;
           if let Err(e) = handle.task.await {
               tracing::warn!(server = %name, "Supervisor task panicked during stop: {e:?}");
           }
           tracing::info!(server = %name, "Server stopped (config removed/changed)");
       }
   }
   ```

4. **Add `apply_config_diff`** — the core reload logic:
   ```rust
   use std::collections::HashSet;
   use crate::config::HubConfig;

   pub async fn apply_config_diff(
       handles: &mut Vec<ServerHandle>,
       old_config: &HubConfig,
       new_config: &HubConfig,
       shutdown: &CancellationToken,
       log_agg: &Arc<LogAggregator>,
   ) -> (usize, usize, usize) {
       let old_names: HashSet<&String> = old_config.servers.keys().collect();
       let new_names: HashSet<&String> = new_config.servers.keys().collect();

       let mut added = 0usize;
       let mut removed = 0usize;
       let mut changed = 0usize;

       // Removed servers: in old but not in new (D-13).
       for name in old_names.difference(&new_names) {
           stop_named_server(handles, name).await;
           removed += 1;
       }

       // New servers: in new but not in old (D-12).
       for name in new_names.difference(&old_names) {
           let cfg = &new_config.servers[*name];
           let handle = start_single_server(name, cfg, shutdown, log_agg).await;
           handles.push(handle);
           added += 1;
       }

       // Existing servers: check if config changed (D-09, D-10, D-11).
       for name in old_names.intersection(&new_names) {
           let old_cfg = &old_config.servers[*name];
           let new_cfg = &new_config.servers[*name];
           if old_cfg != new_cfg {
               // Changed — stop old, start new (D-11).
               stop_named_server(handles, name).await;
               let handle = start_single_server(name, new_cfg, shutdown, log_agg).await;
               handles.push(handle);
               changed += 1;
           }
           // If equal: skip entirely (D-10).
       }

       (added, removed, changed)
   }
   ```
</action>

<acceptance_criteria>
- grep: `pub async fn start_single_server` in src/supervisor.rs
- grep: `pub async fn stop_named_server` in src/supervisor.rs
- grep: `pub async fn apply_config_diff` in src/supervisor.rs
- grep: `old_names.difference(&new_names)` in src/supervisor.rs
- grep: `new_names.difference(&old_names)` in src/supervisor.rs
- grep: `old_names.intersection(&new_names)` in src/supervisor.rs
- grep: `old_cfg != new_cfg` in src/supervisor.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 3: Implement SIGHUP handler and Reload command

<task id="03-04-03">
<read_first>
- src/main.rs (async_main, daemon mode path — from 03-03-04)
- src/supervisor.rs (apply_config_diff — from Task 2)
- src/control.rs (DaemonRequest::Reload — placeholder from 03-03-03)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 6 — SIGHUP handler)
- .planning/phases/03-mcp-introspection-daemon-mode/03-CONTEXT.md (D-09 through D-13)
</read_first>

<action>
1. **Add `Reload` CLI command** in `src/cli.rs`:
   ```rust
   /// Reload config from disk (SIGHUP equivalent).
   Reload,
   ```

2. **In daemon mode (main.rs)**, add SIGHUP handler to the main event loop:
   ```rust
   #[cfg(unix)]
   {
       use tokio::signal::unix::{signal, SignalKind};
       let mut sighup = signal(SignalKind::hangup())?;
       let mut sigterm = signal(SignalKind::terminate())?;

       loop {
           tokio::select! {
               _ = sighup.recv() => {
                   tracing::info!("SIGHUP received — reloading config");
                   handle_reload(
                       &daemon_state,
                       &cli,
                       &mut current_config,
                       &shutdown,
                       &log_agg,
                   ).await;
               }
               _ = sigterm.recv() => {
                   tracing::info!("SIGTERM received — shutting down daemon");
                   break;
               }
               result = tokio::signal::ctrl_c() => {
                   result?;
                   tracing::info!("Ctrl+C received — shutting down daemon");
                   break;
               }
           }
       }
   }
   ```

3. **Implement `handle_reload`** helper in main.rs or as a method:
   ```rust
   async fn handle_reload(
       state: &Arc<control::DaemonState>,
       cli: &Cli,
       current_config: &mut HubConfig,
       shutdown: &CancellationToken,
       log_agg: &Arc<LogAggregator>,
   ) {
       match config::find_and_load_config(cli.config.as_deref()) {
           Ok(new_config) => {
               let mut handles = state.handles.lock().await;
               let (added, removed, changed) = supervisor::apply_config_diff(
                   &mut handles,
                   current_config,
                   &new_config,
                   shutdown,
                   log_agg,
               ).await;
               tracing::info!(
                   added, removed, changed,
                   "Config reload complete"
               );
               *current_config = new_config;
           }
           Err(e) => {
               tracing::error!("Config reload failed: {e}");
           }
       }
   }
   ```

4. **Wire `DaemonRequest::Reload` in control.rs dispatch** — replace the placeholder:
   ```rust
   DaemonRequest::Reload => {
       // Trigger reload via the SIGHUP mechanism.
       // Send SIGHUP to self to trigger the main event loop handler.
       #[cfg(unix)]
       {
           use nix::sys::signal::{kill, Signal};
           use nix::unistd::Pid;
           let pid = Pid::from_raw(std::process::id() as i32);
           match kill(pid, Signal::SIGHUP) {
               Ok(()) => DaemonResponse::ok_empty(),
               Err(e) => DaemonResponse::err(format!("Failed to send SIGHUP: {e}")),
           }
       }
       #[cfg(not(unix))]
       {
           DaemonResponse::err("Reload not supported on this platform".to_string())
       }
   }
   ```

5. **Also add SIGHUP handling in foreground mode** within `run_foreground_loop`:
   - Add `sighup.recv()` as another `select!` arm alongside sigterm and ctrl_c.
   - Call the same reload logic (but with direct access to handles instead of via DaemonState).
   - This allows foreground mode to also support `kill -HUP <pid>` for config reload.

6. **Store `current_config` as mutable** in both daemon and foreground paths so the diff can compare against the latest loaded config.
</action>

<acceptance_criteria>
- grep: `Reload` in src/cli.rs (Commands enum)
- grep: `sighup.recv()` in src/main.rs
- grep: `handle_reload` in src/main.rs
- grep: `apply_config_diff` in src/main.rs
- grep: `DaemonRequest::Reload =>` in src/control.rs (not placeholder)
- grep: `Signal::SIGHUP` in src/control.rs
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 4: Wire all CLI commands to daemon socket client

<task id="03-04-04">
<read_first>
- src/main.rs (Commands::Stop, Restart, Status, Logs — current stub exits)
- src/control.rs (send_daemon_command, DaemonRequest, DaemonResponse — from 03-03-03)
- src/daemon.rs (socket_path — from 03-03-02)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 7 — CLI command changes, D-14 timeouts, D-15 error message)
</read_first>

<action>
In `src/main.rs`, replace the stub exits for all non-Start commands:

1. **`Commands::Stop`**:
   ```rust
   Commands::Stop => {
       let sock = daemon::socket_path()?;
       let response = control::send_daemon_command(
           &sock, &control::DaemonRequest::Stop, 5,
       ).await?;
       if response.ok {
           println!("Daemon stop command sent. Shutting down...");
       } else {
           eprintln!("Error: {}", response.error.unwrap_or_default());
           std::process::exit(1);
       }
       Ok(())
   }
   ```

2. **`Commands::Restart(args)`**:
   ```rust
   Commands::Restart(args) => {
       let sock = daemon::socket_path()?;
       let response = control::send_daemon_command(
           &sock,
           &control::DaemonRequest::Restart { name: args.name.clone() },
           10,
       ).await?;
       if response.ok {
           println!("Restart signal sent to '{}'.", args.name);
       } else {
           eprintln!("Error: {}", response.error.unwrap_or_default());
           std::process::exit(1);
       }
       Ok(())
   }
   ```

3. **`Commands::Status`**:
   ```rust
   Commands::Status => {
       let sock = daemon::socket_path()?;
       let response = control::send_daemon_command(
           &sock, &control::DaemonRequest::Status, 5,
       ).await?;
       if response.ok {
           if let Some(data) = response.data {
               // Pretty-print the status JSON.
               // In the future, format as a proper table matching foreground output.
               println!("{}", serde_json::to_string_pretty(&data)?);
           }
       } else {
           eprintln!("Error: {}", response.error.unwrap_or_default());
           std::process::exit(1);
       }
       Ok(())
   }
   ```

4. **`Commands::Logs(args)`**:
   ```rust
   Commands::Logs(args) => {
       let sock = daemon::socket_path()?;
       let response = control::send_daemon_command(
           &sock,
           &control::DaemonRequest::Logs {
               server: args.server.clone(),
               lines: args.lines,
           },
           5,
       ).await?;
       if response.ok {
           if let Some(data) = response.data {
               if let Some(lines) = data.as_array() {
                   for line in lines {
                       if let Some(s) = line.as_str() {
                           println!("{s}");
                       }
                   }
               }
           }
       } else {
           eprintln!("Error: {}", response.error.unwrap_or_default());
           std::process::exit(1);
       }
       Ok(())
   }
   ```

5. **`Commands::Reload`**:
   ```rust
   Commands::Reload => {
       let sock = daemon::socket_path()?;
       let response = control::send_daemon_command(
           &sock, &control::DaemonRequest::Reload, 5,
       ).await?;
       if response.ok {
           println!("Reload signal sent to daemon.");
       } else {
           eprintln!("Error: {}", response.error.unwrap_or_default());
           std::process::exit(1);
       }
       Ok(())
   }
   ```

6. **Error message when no daemon is running** (D-15):
   The `send_daemon_command` function already includes a clear error message in its connection error:
   `"Cannot connect to daemon socket (...): ...\nIs the daemon running? Start with: mcp-hub start --daemon"`
   This is the standard message when the socket is not connectable.
</action>

<acceptance_criteria>
- grep: `DaemonRequest::Stop` in src/main.rs (Commands::Stop branch)
- grep: `DaemonRequest::Restart` in src/main.rs (Commands::Restart branch)
- grep: `DaemonRequest::Status` in src/main.rs (Commands::Status branch)
- grep: `DaemonRequest::Logs` in src/main.rs (Commands::Logs branch)
- grep: `DaemonRequest::Reload` in src/main.rs (Commands::Reload branch)
- grep: `send_daemon_command` in src/main.rs (appears at least 5 times)
- No `eprintln!.*daemon mode` stubs remaining in src/main.rs
- No `std::process::exit(1)` on the "Phase 3 will implement" message paths
- cargo build succeeds
</acceptance_criteria>
</task>

---

## Task 5: Config reload tests

<task id="03-04-05">
<read_first>
- src/supervisor.rs (apply_config_diff, start_single_server, stop_named_server — from Task 2)
- src/config.rs (ServerConfig with PartialEq — from Task 1, load_config)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 11 — config reload tests)
</read_first>

<action>
Create `tests/config_reload.rs`:

1. **Test: unchanged config — no restarts (D-10)**:
   - Create two identical `HubConfig` values with the same servers.
   - Create mock ServerHandles (or use a minimal server fixture).
   - Call `apply_config_diff`.
   - Assert (added=0, removed=0, changed=0).
   - Assert all handles still present.

2. **Test: add a new server (D-12)**:
   - Old config has servers ["a"]. New config has servers ["a", "b"].
   - Call `apply_config_diff`.
   - Assert (added=1, removed=0, changed=0).
   - Assert handles now has 2 entries.

3. **Test: remove a server (D-13)**:
   - Old config has servers ["a", "b"]. New config has servers ["a"].
   - Call `apply_config_diff`.
   - Assert (added=0, removed=1, changed=0).
   - Assert handles now has 1 entry.
   - Assert the removed server's supervisor task was signaled to stop.

4. **Test: change a server's command (D-11)**:
   - Old config has server "a" with command="old-cmd".
   - New config has server "a" with command="new-cmd".
   - Call `apply_config_diff`.
   - Assert (added=0, removed=0, changed=1).
   - Assert the handle was replaced (old stopped, new started).

5. **Test: PartialEq on ServerConfig**:
   - Two identical ServerConfig instances: assert equal.
   - Change one field (args, env, transport, cwd): assert not equal.
   - Verify each field contributes to equality.

6. **Test: mixed scenario**:
   - Old: ["a", "b", "c"]. New: ["a", "c_modified", "d"].
   - "a" unchanged, "b" removed, "c" changed, "d" added.
   - Assert (added=1, removed=1, changed=1).

Note: These tests need mock server processes. Use the existing ping-responder fixture or a simple `sleep infinity` process. Consider using `tokio::time::pause()` for deterministic timing in tests.
</action>

<acceptance_criteria>
- grep: `fn unchanged_config_no_restarts` in tests/config_reload.rs
- grep: `fn add_new_server` in tests/config_reload.rs
- grep: `fn remove_server` in tests/config_reload.rs
- grep: `fn change_server_command` in tests/config_reload.rs
- grep: `fn server_config_partial_eq` in tests/config_reload.rs
- grep: `fn mixed_config_diff` in tests/config_reload.rs
- cargo test --test config_reload passes
</acceptance_criteria>
</task>

---

## Task 6: Daemon lifecycle integration tests

<task id="03-04-06">
<read_first>
- src/main.rs (daemon mode startup, shutdown — from 03-03-04)
- src/daemon.rs (check_existing_daemon, socket_path, pid_path)
- src/control.rs (send_daemon_command, DaemonRequest)
- .planning/phases/03-mcp-introspection-daemon-mode/03-RESEARCH.md (Section 11 — daemon mode tests)
</read_first>

<action>
Create `tests/cli_daemon.rs`:

1. **Test: `mcp-hub start --daemon` creates socket and PID files** (`#[cfg(unix)]`):
   - Create a temp config file with a simple mock server (e.g., `cat` or the ping-responder).
   - Run `mcp-hub start --daemon -c <config>` via `assert_cmd`.
   - Assert exit code 0.
   - Assert socket file exists at the expected path.
   - Assert PID file exists and contains a valid PID.
   - Clean up: send stop command.

2. **Test: duplicate daemon prevention**:
   - Start a daemon.
   - Try to start a second daemon.
   - Assert second invocation exits with code 1.
   - Assert stderr contains "already running".
   - Clean up first daemon.

3. **Test: `mcp-hub status` returns server info from daemon**:
   - Start daemon with a config containing one server.
   - Wait briefly for startup.
   - Run `mcp-hub status` via `assert_cmd`.
   - Assert exit code 0.
   - Assert stdout contains the server name.

4. **Test: `mcp-hub stop` shuts down daemon**:
   - Start daemon.
   - Run `mcp-hub stop` via `assert_cmd`.
   - Assert exit code 0.
   - Wait briefly.
   - Assert socket file no longer exists.
   - Assert PID file no longer exists.

5. **Test: `mcp-hub restart <name>` via daemon**:
   - Start daemon with a server "test-server".
   - Run `mcp-hub restart test-server` via `assert_cmd`.
   - Assert exit code 0.
   - Assert stdout contains "Restart signal sent".

6. **Test: stale socket cleanup after crash**:
   - Start daemon.
   - Get PID from PID file.
   - Force-kill the daemon process (`kill -9 <pid>`).
   - Run `mcp-hub start --daemon` again.
   - Assert it starts successfully (cleans up stale socket).
   - Clean up.

Note: All daemon tests should use `#[cfg(unix)]` since daemon mode is Unix-only. Use tempdir for config files. Set a unique socket path per test via environment or config override to avoid test interference.
</action>

<acceptance_criteria>
- grep: `fn daemon_creates_socket_and_pid` in tests/cli_daemon.rs
- grep: `fn duplicate_daemon_prevention` in tests/cli_daemon.rs
- grep: `fn status_from_daemon` in tests/cli_daemon.rs
- grep: `fn stop_shuts_down_daemon` in tests/cli_daemon.rs
- grep: `fn restart_via_daemon` in tests/cli_daemon.rs
- grep: `fn stale_socket_cleanup` in tests/cli_daemon.rs
- grep: `cfg(unix)` in tests/cli_daemon.rs
- cargo test --test cli_daemon passes (on Unix)
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
- [ ] ServerConfig derives PartialEq + Eq
- [ ] HubConfig derives PartialEq
- [ ] start_single_server extracted as a public helper
- [ ] stop_named_server properly sends Shutdown and awaits task
- [ ] apply_config_diff handles all four cases: unchanged, added, removed, changed
- [ ] SIGHUP handler in daemon mode triggers config reload
- [ ] SIGHUP handler in foreground mode also works
- [ ] `mcp-hub reload` command sends Reload via socket
- [ ] `mcp-hub stop` connects to daemon socket and sends Stop command
- [ ] `mcp-hub restart <name>` connects to daemon socket and sends Restart
- [ ] `mcp-hub status` connects to daemon socket and prints server info
- [ ] `mcp-hub logs` connects to daemon socket and prints log lines
- [ ] All stubs replaced — no "Phase 3 will implement" messages remain
- [ ] Socket connection failure prints clear "No daemon running" message (D-15)
- [ ] Config reload preserves unchanged servers (no unnecessary restarts)
- [ ] Config reload stops removed servers gracefully
- [ ] Config reload starts newly added servers
- [ ] Config reload restarts changed servers with new config
- [ ] current_config tracked as mutable state for accurate diffs
- [ ] All config reload tests pass
- [ ] All daemon lifecycle tests pass
- [ ] All existing tests still pass
- [ ] No unwrap() in production code (src/)
- [ ] cargo clippy -D warnings passes

### Phase 3 Complete: All 9 Requirements
- [ ] MCP-01: Hub introspects each server on startup (initialize + list requests)
- [ ] MCP-02: JSON-RPC IDs correctly correlated via dispatcher pattern
- [ ] MCP-03: Introspection results stored and visible in status table
- [ ] MCP-04: Transport type explicit in config, v1 implements stdio
- [ ] CFG-03: Config reloadable via SIGHUP or `mcp-hub reload`
- [ ] DMN-02: `mcp-hub start --daemon` daemonizes the process
- [ ] DMN-03: Daemon communicates via Unix domain socket
- [ ] DMN-04: `mcp-hub stop` connects to socket and triggers shutdown
- [ ] DMN-05: Duplicate daemon instances prevented by socket liveness check
</verification>
