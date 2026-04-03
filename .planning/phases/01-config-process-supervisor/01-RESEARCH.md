# Phase 1 Research: Config & Process Supervisor

**Phase:** 01-config-process-supervisor
**Researched:** 2026-04-03
**Requirements covered:** CFG-01, CFG-02, PROC-01, PROC-02, PROC-03, PROC-05, PROC-06, PROC-07, PROC-08, PROC-09, DMN-01

---

## 1. TOML Config Schema Design

### Key Findings

The `[servers.<name>]` map pattern (D-03) deserializes cleanly into a `HashMap<String, ServerConfig>` using serde's standard `#[derive(Deserialize)]`. TOML has first-class support for this shape. The server name becomes the HashMap key — identical to how Cargo's `[dependencies]` table works.

`#[serde(default)]` with a custom function provides field-level defaults without wrapping every optional field in `Option<T>`. For optional overrides (health_check_interval, max_retries, restart_delay from D-05), `Option<T>` is the correct choice — absence in the file means "inherit global default" which is a Phase 2 concern.

Unknown fields in TOML produce a hard error by default with `toml::from_str`. To get forward-compatible "unknown field = warning" behavior (D-06), use `#[serde(deny_unknown_fields)]` only on strict inner structs, and leave the outer struct permissive — or implement a custom deserializer that collects unknown fields into a separate `ignored` bucket. The simplest approach for Phase 1: omit `deny_unknown_fields` entirely (serde ignores unknown fields by default), and emit a warning by deserializing once into a raw `toml::Value`, comparing keys against the known set, then deserializing again into the typed struct.

`env_file` loading (D-04) is NOT something the `toml` crate handles — you must load it manually after config parsing: read the file, parse key=value lines, then merge into the per-server env HashMap with file values taking precedence over inline env values.

### Code Patterns

```rust
// Cargo.toml
// toml = "0.8"
// serde = { version = "1", features = ["derive"] }

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct HubConfig {
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub command: String,                          // required
    #[serde(default)]
    pub args: Vec<String>,                        // optional, default empty
    #[serde(default)]
    pub env: HashMap<String, String>,             // optional inline env
    pub env_file: Option<String>,                 // optional path to .env file
    #[serde(default = "default_transport")]
    pub transport: String,                        // default "stdio"
    pub cwd: Option<String>,                      // optional working directory
    // Phase 2 global defaults — optional overrides here
    pub health_check_interval: Option<u64>,       // seconds
    pub max_retries: Option<u32>,                 // default 10
    pub restart_delay: Option<u64>,               // base delay seconds, default 1
}

fn default_transport() -> String {
    "stdio".to_string()
}

// Parsing and validation entry point
pub fn load_config(path: &std::path::Path) -> anyhow::Result<HubConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;
    
    // First pass: detect unknown fields (forward-compatibility warning)
    let raw: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Invalid TOML in {}", path.display()))?;
    warn_unknown_fields(&raw);
    
    // Second pass: typed deserialization
    let config: HubConfig = toml::from_str(&content)
        .with_context(|| format!("Config schema error in {}", path.display()))?;
    
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &HubConfig) -> anyhow::Result<()> {
    for (name, server) in &config.servers {
        if server.command.is_empty() {
            anyhow::bail!("Server '{}': 'command' must not be empty", name);
        }
        if !matches!(server.transport.as_str(), "stdio" | "http") {
            anyhow::bail!(
                "Server '{}': unknown transport '{}' (expected 'stdio' or 'http')",
                name, server.transport
            );
        }
    }
    Ok(())
}
```

Example TOML (per D-03, D-04):
```toml
[servers.mcp-github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "placeholder" }
env_file = ".env.github"   # values here override inline env

[servers.mcp-filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]
transport = "stdio"
cwd = "/home/user"
max_retries = 5
```

`env_file` merge logic (after config load):
```rust
pub fn resolve_env(server: &ServerConfig) -> anyhow::Result<HashMap<String, String>> {
    let mut resolved = server.env.clone();
    if let Some(path) = &server.env_file {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read env_file: {path}"))?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some((k, v)) = line.split_once('=') {
                // env_file overrides inline env (D-04)
                resolved.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    Ok(resolved)
}
```

### Recommendations

- Keep config loading synchronous — it runs before the Tokio runtime starts, so `std::fs::read_to_string` is correct here.
- `validate_config()` should return `anyhow::Result<()>` and collect all errors before returning, so users see all problems at once rather than fixing one at a time. Use a `Vec<String>` accumulator, then join with newlines.
- Do NOT use `deny_unknown_fields` on `HubConfig` — unknown top-level keys from future versions should warn, not hard error.
- `env_file` parsing must handle Windows line endings (`\r\n`) — use `line.trim()` before processing.

---

## 2. Config File Resolution & Merging

### Key Findings

Decision D-02 specifies: global at `~/.config/mcp-hub/mcp-hub.toml`, local in current directory. The `dirs` crate (`dirs = "5.x"`) provides `dirs::config_dir()` which returns `~/.config` on Linux/macOS and `%APPDATA%` on Windows — correct cross-platform behavior with no manual path construction.

Merge semantics: local `servers` map is unioned with global `servers` map. If the same server name exists in both, local wins (all fields replaced, not field-level merge). This matches Docker Compose's `extends` semantics and is the least surprising behavior.

### Code Patterns

```rust
// Cargo.toml: dirs = "5"

pub fn find_and_load_config() -> anyhow::Result<HubConfig> {
    let global_path = dirs::config_dir()
        .map(|d| d.join("mcp-hub").join("mcp-hub.toml"));
    
    let local_path = std::env::current_dir()
        .ok()
        .map(|d| d.join("mcp-hub.toml"));
    
    let global = global_path
        .filter(|p| p.exists())
        .map(|p| load_config(&p))
        .transpose()?;
    
    let local = local_path
        .filter(|p| p.exists())
        .map(|p| load_config(&p))
        .transpose()?;
    
    match (global, local) {
        (None, None) => anyhow::bail!(
            "No mcp-hub.toml found in current directory or ~/.config/mcp-hub/"
        ),
        (Some(g), None) => Ok(g),
        (None, Some(l)) => Ok(l),
        (Some(mut g), Some(l)) => {
            // Local servers override global by name (D-02)
            g.servers.extend(l.servers);
            Ok(g)
        }
    }
}
```

### Recommendations

- Add a `--config` flag to the CLI to allow explicit config path override — useful for CI, scripting, and testing.
- The `dirs` crate is lighter than `directories` (which provides `ProjectDirs`) — use `dirs` since we only need `config_dir()`.
- If neither file exists and the command is `start`, fail with a clear actionable message: "No mcp-hub.toml found. Create one or run `mcp-hub init`."

---

## 3. Tokio Process Management

### Key Findings

`tokio::process::Command` is the async equivalent of `std::process::Command`. Key behaviors:

**Zombie process prevention (PITFALL #1):** On Unix, a child process that exits without being `wait()`-ed becomes a zombie. `tokio::process` has a background reaper task that handles zombies when `kill_on_drop(false)` (the default), but only if the runtime is still running. The safe pattern is to hold the `Child` handle in a `JoinHandle`-supervised task and always await `.wait()` or `.kill().await` before the task exits — never drop a `Child` silently.

**Process group isolation (D-08, PROC-08):** `Command::process_group(0)` on Unix sets the child's PGID to its own PID, isolating it from the terminal's process group. This prevents Ctrl+C (SIGINT to the terminal's foreground process group) from reaching children — the hub intercepts Ctrl+C and sends SIGTERM itself (D-10). This is the correct pattern per PITFALL #5.

**Pipe ownership:** Set `Stdio::piped()` on stdout AND stderr. Stdout is reserved for the MCP protocol (Phase 3). Stderr is for logs. Both must be continuously drained to prevent the 64KB pipe buffer from filling and blocking the child (PITFALL #2). In Phase 1, stderr drain goes to a simple tokio task that reads and discards (or forwards to tracing). Stdout is taken but parked — the `Child.stdout` handle is stored for Phase 3's MCP client.

**Graceful SIGTERM → SIGKILL sequence (D-07):** Tokio's `Child::kill()` sends SIGKILL directly. To send SIGTERM first, use the `nix` crate: `nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGTERM)`. Then race against a 5-second timer with `tokio::time::timeout`. If the timer fires, call `child.kill().await`.

### Code Patterns

```rust
use tokio::process::{Child, Command};
use tokio::io::{BufReader, AsyncBufReadExt};
use std::process::Stdio;

pub struct SpawnedProcess {
    pub child: Child,
    pub pid: u32,
    // stdout handle kept for MCP client (Phase 3), parked for now
    pub stdout: Option<tokio::process::ChildStdout>,
}

pub fn spawn_server(
    name: &str,
    config: &ServerConfig,
    env: &HashMap<String, String>,
) -> anyhow::Result<SpawnedProcess> {
    let mut cmd = Command::new(&config.command);
    cmd.args(&config.args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);  // we manage cleanup explicitly
    
    // Isolate from terminal process group (PROC-08, PITFALL #5)
    #[cfg(unix)]
    cmd.process_group(0);
    
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }
    
    let mut child = cmd.spawn()
        .with_context(|| format!("Failed to spawn '{}': {}", name, config.command))?;
    
    let pid = child.id().expect("child has PID before first wait");
    let stdout = child.stdout.take();  // take ownership; MCP client claims this later
    
    // Drain stderr continuously to prevent pipe-buffer backpressure (PITFALL #2)
    if let Some(stderr) = child.stderr.take() {
        let server_name = name.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Phase 1: forward to tracing. Phase 2: send to log aggregator.
                tracing::debug!(server = %server_name, "{}", line);
            }
        });
    }
    
    Ok(SpawnedProcess { child, pid, stdout })
}

// Graceful shutdown: SIGTERM, wait 5s, then SIGKILL (D-07, PROC-07)
pub async fn shutdown_process(mut child: Child, pid: u32) -> anyhow::Result<()> {
    // Send SIGTERM to the entire process group (D-08, D-09)
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let pgid = Pid::from_raw(-(pid as i32));  // negative PID = process group
        let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }
    
    // Race: wait for exit vs. 5-second timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        child.wait()
    ).await;
    
    match result {
        Ok(Ok(_)) => Ok(()),  // exited gracefully
        _ => {
            // Timeout or error: SIGKILL
            child.kill().await.ok();
            child.wait().await.ok();  // reap zombie
            Ok(())
        }
    }
}
```

**Killing the entire process group (D-08):**

On Unix, `kill(-pgid, SIGTERM)` sends the signal to every process in the group. Since we set `process_group(0)` on spawn, the child's PGID equals its own PID. So `killpg(Pid::from_raw(child_pid), Signal::SIGTERM)` kills the child and all its descendants.

```rust
// nix = { version = "0.29", features = ["signal", "process"] }
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

pub fn send_sigterm_to_group(pid: u32) {
    let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
}
```

### Recommendations

- Each spawned server should be managed by a dedicated `tokio::task` (the "supervisor task") that owns the `Child` handle and drives the state machine. This is the only safe way to own a `Child` in async code.
- Store `pid: u32` separately before taking `stdin`/`stdout`/`stderr` — `child.id()` returns `None` after the child has been waited.
- On Windows, `Command::process_group()` is a no-op; use Job Objects via the `windows` crate for equivalent behavior. For Phase 1, guard Unix-specific code with `#[cfg(unix)]` and document Windows as "best-effort".
- Always call `child.wait().await` after `child.kill().await` — kill sends SIGKILL but does not reap the zombie.

---

## 4. Signal Handling & Graceful Shutdown

### Key Findings

`tokio::signal::ctrl_c()` is the canonical way to catch SIGINT/Ctrl+C in a Tokio application. It returns a future that completes once. For multi-signal handling (SIGTERM for `mcp-hub stop`, SIGINT for Ctrl+C), use `tokio::signal::unix::signal(SignalKind::terminate())` in combination.

The recommended shutdown architecture uses `tokio_util::sync::CancellationToken` — a clone-able, cancel-able token. All long-running tasks receive a cloned token and select on `token.cancelled()`. The main task calls `token.cancel()` when shutdown is needed.

For Phase 1 (foreground mode only, D-10), the flow is:
1. Install ctrl_c handler before spawning any servers
2. On ctrl_c: call `token.cancel()` — all supervisor tasks see this
3. Each supervisor task catches the cancellation, sends SIGTERM to its child, waits up to 5s, SIGKILL if needed
4. Main task waits for all supervisor task JoinHandles
5. Main task exits 0

### Code Patterns

```rust
// tokio-util = { version = "0.7", features = ["sync"] }
use tokio_util::sync::CancellationToken;
use tokio::task::JoinSet;

pub async fn run(config: HubConfig) -> anyhow::Result<()> {
    let shutdown_token = CancellationToken::new();
    let mut join_set = JoinSet::new();
    
    // Spawn each server in its own supervisor task
    for (name, server_config) in &config.servers {
        let token = shutdown_token.child_token();
        let name = name.clone();
        let server_config = server_config.clone();
        join_set.spawn(async move {
            run_server_supervisor(name, server_config, token).await
        });
    }
    
    // Wait for Ctrl+C, then trigger ordered shutdown
    tokio::signal::ctrl_c().await
        .context("Failed to install Ctrl+C handler")?;
    
    tracing::info!("Ctrl+C received — shutting down all servers");
    shutdown_token.cancel();
    
    // Wait for all supervisor tasks to complete (they do SIGTERM/wait/SIGKILL internally)
    while let Some(result) = join_set.join_next().await {
        if let Err(e) = result {
            tracing::warn!("Supervisor task panicked: {e}");
        }
    }
    
    Ok(())
}

async fn run_server_supervisor(
    name: String,
    config: ServerConfig,
    shutdown: CancellationToken,
) {
    loop {
        let spawned = match spawn_server(&name, &config, &resolve_env(&config).unwrap_or_default()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(server = %name, "Failed to spawn: {e}");
                // backoff and retry handled below
                break;
            }
        };
        
        tokio::select! {
            // Child exited on its own
            status = spawned.child.wait() => {
                tracing::warn!(server = %name, "Exited with {:?}", status);
                // backoff logic (Section 5)
            }
            // Hub is shutting down
            _ = shutdown.cancelled() => {
                shutdown_process(spawned.child, spawned.pid).await.ok();
                return;  // clean exit, no retry
            }
        }
        
        // If we reach here, child crashed — check backoff/fatal state
        // ...
    }
}
```

**Parallel stop for `mcp-hub stop` (D-09):** Collect all running PIDs, send SIGTERM to all simultaneously, then join all wait futures with `futures::future::join_all` or `JoinSet`.

### Recommendations

- Use `CancellationToken` from `tokio-util` over raw `broadcast` channels for shutdown signaling — it is composable (child tokens), clone-able, and integrates cleanly with `tokio::select!`.
- `JoinSet` (tokio 1.21+) is the correct way to track a dynamic set of tasks and drain them on shutdown — it handles panics cleanly and drops handles automatically.
- Do NOT use `tokio::signal::ctrl_c()` more than once — it only fires once per call. Re-register inside a loop if you want to handle multiple Ctrl+C presses (second Ctrl+C = force kill).
- On Unix, also handle SIGTERM for `mcp-hub stop` sent from shell: use `tokio::signal::unix::signal(SignalKind::terminate())` and `tokio::select!` on both signals.

---

## 5. Exponential Backoff & State Machine

### Key Findings

The state machine per server (from ARCHITECTURE.md) is:
```
STOPPED -> STARTING -> RUNNING -> STOPPING -> STOPPED
                    -> BACKOFF  -> STARTING  (crash loop)
                    -> FATAL    (max retries exceeded)
```

Key decisions (D-11, D-12, D-13, D-14):
- Backoff intervals: 1s → 2s → 4s → 8s → 16s → 32s → 60s (capped)
- Jitter: ±30% per interval
- Fatal threshold: 10 consecutive failures
- Backoff reset: after 60 continuous seconds in RUNNING state
- Fatal clears on fresh `mcp-hub start` (D-14)

PITFALL #3 highlights the thundering herd problem — jitter is mandatory when multiple servers crash simultaneously.

In Rust, process state machines are best represented as an enum (not the typestate pattern) when the state needs to be stored in a `HashMap` and inspected at runtime. The typestate pattern (state encoded in the type system) works well for linear protocols but becomes awkward for a dynamic process registry where states change at runtime.

### Code Patterns

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Backoff { attempt: u32, until: std::time::Instant },
    Fatal,
    Stopping,
}

pub struct BackoffConfig {
    pub base_delay_secs: f64,      // default 1.0
    pub max_delay_secs: f64,       // default 60.0
    pub jitter_factor: f64,        // default 0.3 (±30%)
    pub max_attempts: u32,         // default 10
    pub stable_window_secs: u64,   // default 60 — reset after this many seconds running
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            base_delay_secs: 1.0,
            max_delay_secs: 60.0,
            jitter_factor: 0.3,
            max_attempts: 10,
            stable_window_secs: 60,
        }
    }
}

pub fn compute_backoff_delay(attempt: u32, config: &BackoffConfig) -> std::time::Duration {
    use rand::Rng;
    
    // Cap attempt to prevent u32 overflow on long-running failures
    let capped_attempt = attempt.min(10);
    let base = config.base_delay_secs * (2u32.pow(capped_attempt) as f64);
    let capped = base.min(config.max_delay_secs);
    
    // ±30% jitter: multiply by random in [0.7, 1.3]
    let jitter = rand::thread_rng().gen_range(
        (1.0 - config.jitter_factor)..=(1.0 + config.jitter_factor)
    );
    
    let final_secs = (capped * jitter).max(0.1);  // floor at 100ms
    std::time::Duration::from_secs_f64(final_secs)
}

// The backoff loop inside the supervisor task
async fn run_with_backoff(
    name: &str,
    config: &ServerConfig,
    shutdown: CancellationToken,
    state_tx: tokio::sync::watch::Sender<ProcessState>,
) {
    let backoff_cfg = BackoffConfig::default();
    let mut consecutive_failures: u32 = 0;
    
    loop {
        // Spawn attempt
        let _ = state_tx.send(ProcessState::Starting);
        
        let spawned = match spawn_server(name, config, &HashMap::new()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(server = %name, "Spawn failed: {e}");
                consecutive_failures += 1;
                if consecutive_failures >= backoff_cfg.max_attempts {
                    let _ = state_tx.send(ProcessState::Fatal);
                    tracing::error!(server = %name, "Marked Fatal after {} failures", consecutive_failures);
                    return;
                }
                // Still retry — apply backoff
                let delay = compute_backoff_delay(consecutive_failures - 1, &backoff_cfg);
                let _ = state_tx.send(ProcessState::Backoff {
                    attempt: consecutive_failures,
                    until: std::time::Instant::now() + delay,
                });
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.cancelled() => return,
                }
                continue;
            }
        };
        
        let pid = spawned.pid;
        let started_at = std::time::Instant::now();
        let _ = state_tx.send(ProcessState::Running);
        
        tokio::select! {
            status = spawned.child.wait() => {
                let ran_for = started_at.elapsed().as_secs();
                
                // Reset backoff counter if server ran long enough (D-13)
                if ran_for >= backoff_cfg.stable_window_secs {
                    consecutive_failures = 0;
                }
                
                consecutive_failures += 1;
                tracing::warn!(server = %name, "Exited after {}s (attempt {})", ran_for, consecutive_failures);
                
                if consecutive_failures >= backoff_cfg.max_attempts {
                    let _ = state_tx.send(ProcessState::Fatal);
                    return;
                }
                
                let delay = compute_backoff_delay(consecutive_failures - 1, &backoff_cfg);
                let _ = state_tx.send(ProcessState::Backoff {
                    attempt: consecutive_failures,
                    until: std::time::Instant::now() + delay,
                });
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.cancelled() => return,
                }
            }
            _ = shutdown.cancelled() => {
                let _ = state_tx.send(ProcessState::Stopping);
                shutdown_process(spawned.child, pid).await.ok();
                let _ = state_tx.send(ProcessState::Stopped);
                return;
            }
        }
    }
}
```

**State observation:** Use `tokio::sync::watch` for per-server state — a single-value channel where receivers always see the latest state. This is ideal for the status table printed at the end of `mcp-hub start`.

### Recommendations

- Add the `rand` crate for jitter: `rand = "0.9"` (just `rand::thread_rng().gen_range()`).
- Cap `attempt` at `10` in the backoff formula before the `pow` to prevent integer overflow on very long-running servers with intermittent crashes.
- `consecutive_failures` counts CONSECUTIVE failures, not total. A server that ran for 10 minutes and crashed 3 times overnight is not in the same situation as one that crashed 3 times in 5 seconds.
- Do NOT use an external backoff crate (`backon`, `exponential-backoff`) — the logic is 15 lines and the decisions (jitter formula, reset condition) are specific to this project.
- Fatal state is per-session only (D-14) — the field lives in the running supervisor task's local state, not in a persistent file. Fresh `mcp-hub start` naturally creates new tasks with fresh state.

---

## 6. CLI Design with clap Derive

### Key Findings

clap 4.x derive API is the standard. The full pattern for `mcp-hub` Phase 1 requires:
- Top-level `struct Cli` with global flags (`--no-color`, `-v`/`-vv`, `--config`)
- `enum Commands` for subcommands
- Each subcommand as a variant with its own `#[derive(Args)]` struct for per-command flags

Phase 1 subcommands: `start`, `stop`, `restart <name>`, and the implicit `--version`/`--help`.

### Code Patterns

```rust
use clap::{Parser, Subcommand, Args};

#[derive(Parser, Debug)]
#[command(
    name = "mcp-hub",
    version,
    about = "PM2 for MCP servers — manage, monitor, and configure your MCP servers",
    long_about = None,
)]
pub struct Cli {
    /// Disable colored output
    #[arg(long, global = true, env = "NO_COLOR")]
    pub no_color: bool,
    
    /// Increase verbosity (-v for events, -vv for debug)
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    
    /// Path to config file (default: search local + ~/.config/mcp-hub/mcp-hub.toml)
    #[arg(long, short = 'c', global = true, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
    
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start all configured MCP servers
    Start(StartArgs),
    /// Stop all running servers (SIGTERM → 5s → SIGKILL)
    Stop,
    /// Restart a specific server by name
    Restart(RestartArgs),
}

#[derive(Args, Debug)]
pub struct StartArgs {
    /// Output format for status table
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Args, Debug)]
pub struct RestartArgs {
    /// Name of the server to restart (as defined in mcp-hub.toml)
    pub name: String,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}
```

**Global flag inheritance:** The `global = true` attribute propagates flags down to all subcommands — a user can write `mcp-hub --no-color start` or `mcp-hub start --no-color` with identical effect.

**Environment variable auto-binding:** `env = "NO_COLOR"` makes clap automatically read from the `NO_COLOR` environment variable (the standard from no-color.org). This is zero extra code.

### Recommendations

- Use `clap::ArgAction::Count` for verbosity so `-v` gives 1 and `-vv` gives 2, which maps cleanly to tracing levels.
- Add `#[command(propagate_version = true)]` to the top-level struct — this makes `--version` available on all subcommands, not just the root.
- Phase 1 does NOT need a `status` subcommand — that is PROC-04, deferred to Phase 2. The `start` command already prints a status table after launching.
- Exit codes: implement via `std::process::exit(code)` at the top level after matching on `anyhow::Result` — `0` for success, `1` for errors. Exit code `2` (warnings only) is not needed in Phase 1.

---

## 7. Terminal Output: Colors & Tables

### Key Findings

**Colors:** `owo-colors` is the recommended crate for Phase 1. It is:
- Zero-allocation, no_std-compatible
- Handles `NO_COLOR` env var, `CLICOLOR_FORCE`, and `TERM` checking
- Has an `if_supports_color()` method that evaluates all these conditions at call time
- Integrates with `std::io::IsTerminal` for TTY detection

Alternative: `termcolor` (BurntSushi) is battle-tested and used by ripgrep. It does NOT do TTY detection itself but works with `std::io::IsTerminal`. For a simple status table, `owo-colors` is simpler.

**TTY detection:** `std::io::IsTerminal` (stable since Rust 1.70) is the standard way. No external crate needed.

**Tables:** `comfy-table` is the right choice. It supports:
- Dynamic column widths with content wrapping
- ANSI color content styling in cells
- Multiple preset styles (clean, borders, etc.)
- No external process or heavy dependencies

`tabled` is more feature-rich but overkill for a status table with 4 columns.

### Code Patterns

```rust
// Cargo.toml: owo-colors = { version = "4", features = ["supports-colors"] }
// comfy-table = "7"

use owo_colors::OwoColorize;
use std::io::IsTerminal;

fn use_colors(no_color_flag: bool) -> bool {
    !no_color_flag && std::io::stdout().is_terminal()
}

fn print_status_table(
    servers: &[(String, ProcessState, Option<u32>)],
    color: bool,
) {
    use comfy_table::{Table, Cell, Color};
    
    let mut table = Table::new();
    table.set_header(["Name", "State", "PID"]);
    
    for (name, state, pid) in servers {
        let state_cell = if color {
            match state {
                ProcessState::Running => Cell::new("running").fg(Color::Green),
                ProcessState::Starting => Cell::new("starting").fg(Color::Yellow),
                ProcessState::Fatal => Cell::new("fatal").fg(Color::Red),
                ProcessState::Backoff { attempt, .. } => {
                    Cell::new(format!("backoff ({})", attempt)).fg(Color::Yellow)
                }
                ProcessState::Stopped => Cell::new("stopped").fg(Color::DarkGrey),
                ProcessState::Stopping => Cell::new("stopping").fg(Color::Yellow),
            }
        } else {
            Cell::new(format!("{:?}", state))
        };
        
        table.add_row([
            Cell::new(name),
            state_cell,
            Cell::new(pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string())),
        ]);
    }
    
    println!("{table}");
}
```

**D-16 implementation:** `--no-color` flag + TTY detection covers all cases:
1. `--no-color` flag → always plain text
2. stdout not a TTY (piped to file/other process) → `IsTerminal` returns false → plain text
3. `NO_COLOR` env var set → `owo-colors` respects it automatically

### Recommendations

- Compose the color decision once at CLI parse time into a `bool` and pass it down — do not re-check TTY on every render call.
- `comfy-table` does not need to be initialized with ANSI support — use `comfy_table::Cell::fg()` for coloring individual cells, which works when the content string includes ANSI codes.
- For verbosity levels (D-17): configure `tracing-subscriber` with `EnvFilter` where `-v` = INFO, `-vv` = DEBUG, default = WARN. Use `tracing::warn!` for errors-only default output.

---

## 8. Process Group Management (Unix/nix crate)

### Key Findings

Two approaches exist for killing a process group:

**Approach A — nix::sys::signal::killpg:** Sends a signal to an entire process group by PGID. Clean, explicit. Available in `nix = "0.29"` with `features = ["signal"]`.

**Approach B — tokio::process::Command::process_group(0) + Child::kill():** `process_group(0)` sets the child's PGID to its own PID on spawn. Then `Child::kill()` only kills the direct child, not its descendants. Need `killpg` for descendants.

The correct combination:
1. `Command::process_group(0)` on spawn — isolates from terminal, sets PGID = child PID
2. `killpg(child_pid, SIGTERM)` from nix — kills entire group including descendants
3. Wait 5s
4. `Child::kill().await` (SIGKILL the direct child if still alive, which also kills group members who ignored SIGTERM)

**Windows:** No equivalent to `process_group(0)` — use Windows Job Objects to achieve process tree containment. For Phase 1, guard with `#[cfg(unix)]` and document Windows as "SIGTERM not sent to process groups".

### Code Patterns

```rust
#[cfg(unix)]
pub fn kill_process_group(pgid: u32, signal: nix::sys::signal::Signal) {
    use nix::sys::signal::killpg;
    use nix::unistd::Pid;
    // pgid == child's PID (set via process_group(0))
    if let Err(e) = killpg(Pid::from_raw(pgid as i32), signal) {
        tracing::warn!("killpg({pgid}, {signal:?}) failed: {e}");
    }
}

#[cfg(windows)]
pub fn kill_process_group(pgid: u32, _signal: ()) {
    // Windows: TerminateProcess is handled by Child::kill()
    // Job Object support deferred to a future phase
}
```

### Recommendations

- The `nix` feature flags matter: `nix = { version = "0.29", features = ["signal", "process"] }` — `signal` for `killpg`, `process` for `getpgid`/`setpgid`.
- `killpg` with the child's PID as PGID correctly kills all processes that were spawned with `process_group(0)` — the PGID equals the PID of the process that called `process_group(0)`.
- On macOS, behavior is identical to Linux for this use case.
- Log killpg errors at WARN level, not ERROR — a process may have already exited between the SIGTERM send and the killpg call, producing ESRCH (no such process), which is benign.

---

## 9. Hub State Architecture

### Key Findings

Phase 1 needs a lightweight shared state struct that:
- Holds per-server `ProcessState` and `pid: Option<u32>`
- Is readable by the CLI output layer for the status table after `mcp-hub start`
- Is writable by supervisor tasks as they transition states

`tokio::sync::watch` per server is the right primitive for Phase 1: each supervisor task owns a `watch::Sender<ProcessState>`, and the CLI collects all `watch::Receiver<ProcessState>` values to build the status table. No shared HashMap, no RwLock contention.

For Phase 2+, this evolves into a full `Arc<RwLock<HubState>>` (documented in ARCHITECTURE.md). For Phase 1, keep it simple.

### Code Patterns

```rust
use tokio::sync::watch;

pub struct ServerHandle {
    pub name: String,
    pub state_rx: watch::Receiver<ProcessState>,
    pub pid_rx: watch::Receiver<Option<u32>>,
    pub task: tokio::task::JoinHandle<()>,
}

// In the start command handler:
pub async fn cmd_start(config: HubConfig, no_color: bool) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let mut handles = Vec::new();
    
    for (name, server_config) in &config.servers {
        let (state_tx, state_rx) = watch::channel(ProcessState::Stopped);
        let (pid_tx, pid_rx) = watch::channel(None::<u32>);
        
        let token = shutdown.child_token();
        let name_clone = name.clone();
        let cfg = server_config.clone();
        
        let task = tokio::spawn(async move {
            run_with_backoff_and_watch(&name_clone, &cfg, token, state_tx, pid_tx).await;
        });
        
        handles.push(ServerHandle { name: name.clone(), state_rx, pid_rx, task });
    }
    
    // Wait for all servers to reach Running or Fatal/Backoff
    // Then print status table (D-15)
    wait_for_initial_state(&mut handles).await;
    print_status_table_from_handles(&handles, no_color);
    
    // Then block on Ctrl+C
    tokio::signal::ctrl_c().await?;
    shutdown.cancel();
    
    for handle in handles {
        handle.task.await.ok();
    }
    
    Ok(())
}
```

### Recommendations

- `watch::Receiver::borrow()` is a non-async, non-blocking read — safe to call from sync context for the status table.
- Phase 1 can skip the full `Arc<RwLock<HubState>>` — that is needed when the web UI (Phase 4) needs concurrent read access. For Phase 1, collect state from watch receivers in the main task.
- Use `watch::channel(initial_value)` not `broadcast` — `watch` keeps only the latest value (exactly what process state needs); `broadcast` keeps N messages and requires active reading to not lag.

---

## 10. File Structure for Phase 1

Based on ARCHITECTURE.md and phase scope:

```
mcp-hub/
├── src/
│   ├── main.rs          # tokio::main, parse CLI, dispatch to commands
│   ├── cli.rs           # clap Cli, Commands enum, args structs
│   ├── config.rs        # HubConfig, ServerConfig, load_config(), find_and_load_config()
│   ├── types.rs         # ProcessState enum, BackoffConfig, SpawnedProcess
│   ├── supervisor.rs    # spawn_server(), shutdown_process(), run_with_backoff(), compute_backoff_delay()
│   └── output.rs        # print_status_table(), color helpers, use_colors()
├── tests/
│   ├── fixtures/
│   │   ├── valid.toml           # clean config, passes all validation
│   │   ├── invalid-missing-command.toml
│   │   ├── invalid-bad-transport.toml
│   │   └── unknown-fields.toml  # should warn, not error
│   ├── config_test.rs
│   └── supervisor_test.rs
├── Cargo.toml
├── Cargo.lock
├── .github/workflows/ci.yml
├── README.md
├── CONTRIBUTING.md
└── LICENSE
```

`state.rs` and `logs.rs` are Phase 2 additions. `mcp/` is Phase 3. `web/` is Phase 4. Keep Phase 1 files minimal.

---

## Validation Architecture

How to verify Phase 1 was implemented correctly:

### Unit Tests

| Test | File | Asserts |
|------|------|---------|
| `parse_valid_config` | `tests/config_test.rs` | valid.toml parses to HubConfig with expected server names and fields |
| `parse_missing_command_errors` | `tests/config_test.rs` | `command = ""` returns Err with message containing server name |
| `parse_bad_transport_errors` | `tests/config_test.rs` | `transport = "grpc"` returns Err with clear message |
| `parse_unknown_fields_warns` | `tests/config_test.rs` | file with `unknown_key = true` parses successfully (warning emitted) |
| `env_file_overrides_inline` | `tests/config_test.rs` | env_file KEY=newval overrides env = { KEY = "oldval" } |
| `local_overrides_global` | `tests/config_test.rs` | server in both configs uses local version |
| `backoff_delay_increases` | `tests/supervisor_test.rs` | delay(attempt=0) < delay(attempt=3) < delay(attempt=6) <= 60s |
| `backoff_jitter_in_range` | `tests/supervisor_test.rs` | over 100 samples, all delays within ±30% of base |
| `backoff_cap_at_60s` | `tests/supervisor_test.rs` | delay(attempt=20) <= 60s |

### Integration Tests (assert_cmd)

| Test | Asserts |
|------|---------|
| `start_launches_echo_server` | spawns a `sleep 100` server, verifies PID in status output, status table shows "running" |
| `stop_kills_children` | after stop, verify PIDs no longer in `ps` output, no zombies |
| `restart_named_server` | PIDs differ before and after restart; other servers keep running |
| `ctrl_c_clean_shutdown` | send SIGINT to hub process, verify all children exit within 6s |
| `fatal_after_10_failures` | spawn a `false` server (exits immediately), verify "fatal" in status after 10 attempts |
| `missing_config_exits_nonzero` | run in empty dir with no config, verify exit code 1 and error message |
| `bad_toml_exits_nonzero` | config with syntax error, verify exit code 1 and line/column in error |
| `no_zombie_processes` | start/stop loop 10 times, verify `ps -e | wc -l` stays constant |

### Manual Smoke Tests (from ROADMAP.md success criteria)

1. `mcp-hub start` with valid config → status table printed, all servers show "running"
2. Config with typo → clear error printed, exit code 1
3. `mcp-hub stop` → no zombie processes in `ps aux`, exit 0
4. `mcp-hub restart mcp-github` → only that server restarts, PID changes, others unchanged
5. Server that exits immediately → backoff delays visible in `-v` output, "fatal" after 10 attempts
6. Ctrl+C in terminal → all children exit within 5s, hub exits 0

---

## RESEARCH COMPLETE
