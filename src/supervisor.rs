use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context as _;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio_util::sync::CancellationToken;

use crate::config::{resolve_env, HubConfig, ServerConfig};
use crate::types::{BackoffConfig, ProcessState};

// ─────────────────────────────────────────────────────────────────────────────
// Task 1: Process spawning and shutdown
// ─────────────────────────────────────────────────────────────────────────────

/// A child process that has been spawned by [`spawn_server`].
pub struct SpawnedProcess {
    /// The Tokio child handle. Owned by this struct and must be explicitly awaited.
    pub child: tokio::process::Child,
    /// The OS-assigned PID, captured before any `take()` calls drain it.
    pub pid: u32,
    /// Reserved for the Phase 3 MCP client — the stdout pipe handle.
    /// In Phase 1, callers that do not consume this should spawn a drain task.
    pub stdout: Option<ChildStdout>,
}

/// Spawn a child process for an MCP server.
///
/// - Pipes stdin, stdout, and stderr (prevents pipe-buffer backpressure — PITFALL #2).
/// - On Unix, puts the child in its own process group so Ctrl+C is not forwarded
///   directly to the child (PROC-08, PITFALL #5).
/// - Spawns a dedicated tokio task to drain stderr line-by-line into `tracing::debug!`.
/// - Takes stdout and stores it for Phase 3 MCP client handoff.
/// - Does NOT call `kill_on_drop(true)` — cleanup is always explicit.
pub fn spawn_server(
    name: &str,
    config: &ServerConfig,
    env: &HashMap<String, String>,
) -> anyhow::Result<SpawnedProcess> {
    let mut cmd = tokio::process::Command::new(&config.command);
    cmd.args(&config.args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false); // explicit cleanup only

    // Isolate from terminal process group (PROC-08, PITFALL #5).
    #[cfg(unix)]
    cmd.process_group(0);

    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn '{}': {}", name, config.command))?;

    // Capture PID before any take() calls drain it.
    let pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("Failed to get PID for '{name}'"))?;

    // Take stdout for Phase 3 MCP client handoff; drain in Phase 1 to prevent blocking.
    let stdout = child.stdout.take();

    // Drain stderr continuously to prevent pipe-buffer backpressure (PITFALL #2).
    if let Some(stderr) = child.stderr.take() {
        let server_name = name.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(server = %server_name, "{}", line);
            }
        });
    }

    Ok(SpawnedProcess { child, pid, stdout })
}

/// Gracefully shut down a child process: SIGTERM → 5 s wait → SIGKILL.
///
/// On Unix, SIGTERM is sent to the **entire process group** via `killpg` so that
/// child processes spawned by the server also receive it (D-07, D-08, PROC-07).
///
/// Always calls `child.wait()` after any kill to reap zombies (PROC-09).
pub async fn shutdown_process(mut child: tokio::process::Child, pid: u32) -> anyhow::Result<()> {
    // Send SIGTERM to the entire process group (D-08).
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;

        if let Err(err) = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM) {
            // ESRCH means the process already exited — benign.
            tracing::warn!(
                pid,
                "killpg(SIGTERM) failed (process may have exited): {err}"
            );
        }
    }

    // Race child.wait() against a 5-second timeout (D-07).
    let result = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

    match result {
        Ok(Ok(_)) => {
            // Child exited within the grace period.
            Ok(())
        }
        _ => {
            // Timeout or wait error — escalate to SIGKILL.
            tracing::warn!(pid, "Grace period elapsed; sending SIGKILL");
            child.kill().await.ok();
            child.wait().await.ok(); // reap zombie (PROC-09)
            Ok(())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2: Exponential backoff and run_server_supervisor
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the delay before the next restart attempt.
///
/// Formula: `base * 2^attempt`, capped at `max_delay_secs`, with ±`jitter_factor` random noise.
/// The attempt counter is capped at 10 before the pow to prevent integer overflow (PITFALL #3).
/// The result is floored at 100 ms.
pub fn compute_backoff_delay(attempt: u32, config: &BackoffConfig) -> Duration {
    use rand::Rng as _;

    // Cap attempt to prevent u32 overflow in 2^attempt.
    let capped_attempt = attempt.min(10);
    let base = config.base_delay_secs * (2u32.pow(capped_attempt) as f64);
    let capped = base.min(config.max_delay_secs);

    // ±jitter_factor (default ±30%): multiply by a random scalar in
    // [1 - jitter_factor, 1 + jitter_factor].
    let jitter =
        rand::rng().random_range((1.0 - config.jitter_factor)..=(1.0 + config.jitter_factor));

    let final_secs = (capped * jitter).max(0.1); // floor at 100 ms
    Duration::from_secs_f64(final_secs)
}

/// Command sent to a running supervisor task via its `cmd_rx` channel.
pub enum SupervisorCommand {
    /// Stop the managed server and exit the supervisor loop.
    Shutdown,
    /// Stop the managed server, reset backoff state, and restart from scratch.
    ///
    /// In Phase 1, this is sent from the foreground stdin command reader when
    /// the user types `restart <name>`. In Phase 3, it will also be wired from
    /// the daemon-mode CLI restart subcommand.
    Restart,
}

/// Long-running task that manages the lifecycle of a single MCP server.
///
/// State transitions:
/// ```text
/// Stopped -> Starting -> Running -> (crash) -> Backoff -> Starting …
///                     -> (shutdown) -> Stopping -> Stopped
///                     -> (max failures) -> Fatal
/// ```
///
/// The task terminates when:
/// - A [`SupervisorCommand::Shutdown`] is received, OR
/// - The [`CancellationToken`] is cancelled, OR
/// - The server enters the [`ProcessState::Fatal`] state.
pub async fn run_server_supervisor(
    name: String,
    config: ServerConfig,
    shutdown: CancellationToken,
    mut cmd_rx: tokio::sync::mpsc::Receiver<SupervisorCommand>,
    state_tx: tokio::sync::watch::Sender<(ProcessState, Option<u32>)>,
) {
    let backoff_cfg = BackoffConfig::default();
    let mut consecutive_failures: u32 = 0;

    // Resolve environment variables once — env_file is static for the lifetime of the process.
    let env = match resolve_env(&config) {
        Ok(e) => e,
        Err(err) => {
            tracing::error!(server = %name, "Failed to resolve env: {err}");
            let _ = state_tx.send((ProcessState::Fatal, None));
            return;
        }
    };

    loop {
        // ── Starting ──────────────────────────────────────────────────────────
        let _ = state_tx.send((ProcessState::Starting, None));

        let spawned = match spawn_server(&name, &config, &env) {
            Ok(s) => s,
            Err(err) => {
                tracing::error!(server = %name, "Spawn failed: {err}");
                consecutive_failures += 1;

                if consecutive_failures >= backoff_cfg.max_attempts {
                    tracing::error!(
                        server = %name,
                        "Marked Fatal after {} consecutive failures",
                        consecutive_failures
                    );
                    let _ = state_tx.send((ProcessState::Fatal, None));
                    return;
                }

                let delay = compute_backoff_delay(consecutive_failures - 1, &backoff_cfg);
                let _ = state_tx.send((
                    ProcessState::Backoff {
                        attempt: consecutive_failures,
                        until: std::time::Instant::now() + delay,
                    },
                    None,
                ));

                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.cancelled() => return,
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(SupervisorCommand::Shutdown) | None => return,
                            Some(SupervisorCommand::Restart) => {
                                consecutive_failures = 0;
                                continue;
                            }
                        }
                    }
                }
                continue;
            }
        };

        // ── Running ───────────────────────────────────────────────────────────
        let pid = spawned.pid;
        let started_at = std::time::Instant::now();
        let _ = state_tx.send((ProcessState::Running, Some(pid)));

        // Drain stdout in Phase 1 to prevent pipe-buffer backpressure (PITFALL #2).
        // Phase 3 will hand `stdout` to the MCP client instead of draining here.
        if let Some(stdout) = spawned.stdout {
            let drain_name = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(server = %drain_name, "[stdout] {}", line);
                }
            });
        }

        let mut child = spawned.child;

        tokio::select! {
            // Child exited on its own (crash or clean exit).
            status = child.wait() => {
                let ran_for = started_at.elapsed().as_secs();
                tracing::warn!(
                    server = %name,
                    "Exited after {}s with status {:?}",
                    ran_for,
                    status
                );

                // Reset backoff counter if the server ran stably long enough (D-13).
                if ran_for >= backoff_cfg.stable_window_secs {
                    consecutive_failures = 0;
                }

                consecutive_failures += 1;

                if consecutive_failures >= backoff_cfg.max_attempts {
                    tracing::error!(
                        server = %name,
                        "Marked Fatal after {} consecutive failures",
                        consecutive_failures
                    );
                    let _ = state_tx.send((ProcessState::Fatal, None));
                    return;
                }

                let delay = compute_backoff_delay(consecutive_failures - 1, &backoff_cfg);
                let _ = state_tx.send((
                    ProcessState::Backoff {
                        attempt: consecutive_failures,
                        until: std::time::Instant::now() + delay,
                    },
                    None,
                ));

                // Wait out the backoff delay, interruptible by shutdown or Restart command.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.cancelled() => return,
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(SupervisorCommand::Shutdown) | None => return,
                            Some(SupervisorCommand::Restart) => {
                                consecutive_failures = 0;
                            }
                        }
                    }
                }
            }

            // Hub-level shutdown (Ctrl+C or SIGTERM).
            _ = shutdown.cancelled() => {
                let _ = state_tx.send((ProcessState::Stopping, Some(pid)));
                shutdown_process(child, pid).await.ok();
                let _ = state_tx.send((ProcessState::Stopped, None));
                return;
            }

            // Explicit command from the CLI dispatcher.
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SupervisorCommand::Restart) => {
                        tracing::info!(server = %name, "Restart requested");
                        let _ = state_tx.send((ProcessState::Stopping, Some(pid)));
                        shutdown_process(child, pid).await.ok();
                        consecutive_failures = 0;
                        // Continue the outer loop — will re-spawn.
                    }
                    Some(SupervisorCommand::Shutdown) | None => {
                        let _ = state_tx.send((ProcessState::Stopping, Some(pid)));
                        shutdown_process(child, pid).await.ok();
                        let _ = state_tx.send((ProcessState::Stopped, None));
                        return;
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3: Hub orchestrator
// ─────────────────────────────────────────────────────────────────────────────

/// A handle to a running server supervisor task.
pub struct ServerHandle {
    /// The name of the server as defined in the config file.
    pub name: String,
    /// Receiver for the server's latest `(ProcessState, Option<pid>)` snapshot.
    pub state_rx: tokio::sync::watch::Receiver<(ProcessState, Option<u32>)>,
    /// Sender for dispatching [`SupervisorCommand`]s to the supervisor task.
    pub cmd_tx: tokio::sync::mpsc::Sender<SupervisorCommand>,
    /// JoinHandle for the supervisor task itself.
    pub task: tokio::task::JoinHandle<()>,
}

/// Spawn a supervisor task for every server in the config.
///
/// Returns one [`ServerHandle`] per server so the caller can observe state and
/// send commands.
pub async fn start_all_servers(
    config: &HubConfig,
    shutdown: CancellationToken,
) -> Vec<ServerHandle> {
    let mut handles = Vec::with_capacity(config.servers.len());

    for (name, server_config) in &config.servers {
        let (state_tx, state_rx) = tokio::sync::watch::channel((ProcessState::Stopped, None));
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(8);
        let token = shutdown.child_token();
        let name_owned = name.clone();
        let cfg = server_config.clone();

        let task = tokio::spawn(async move {
            run_server_supervisor(name_owned, cfg, token, cmd_rx, state_tx).await;
        });

        handles.push(ServerHandle {
            name: name.clone(),
            state_rx,
            cmd_tx,
            task,
        });
    }

    handles
}

/// Wait until every server has left the `Stopped`/`Starting` transient states,
/// or until `timeout` elapses.
///
/// Used by the `start` command to know when to print the initial status table (D-15).
pub async fn wait_for_initial_states(handles: &mut [ServerHandle], timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;

    for handle in handles.iter_mut() {
        loop {
            let (state, _) = handle.state_rx.borrow().clone();
            match state {
                ProcessState::Stopped | ProcessState::Starting => {
                    // Still transitioning — wait for the next change.
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    let changed = tokio::time::timeout(remaining, handle.state_rx.changed()).await;
                    if changed.is_err() {
                        // Timeout — move on.
                        break;
                    }
                }
                _ => break, // Running, Backoff, Fatal, Stopping
            }
        }
    }
}

/// Send a [`SupervisorCommand::Shutdown`] to every server simultaneously and
/// await all supervisor tasks to completion (D-09: parallel stop).
pub async fn stop_all_servers(handles: Vec<ServerHandle>) {
    // Send Shutdown to all handles in parallel — fire and forget the send errors
    // (the task may have already exited if the server went Fatal).
    for handle in &handles {
        let _ = handle.cmd_tx.send(SupervisorCommand::Shutdown).await;
    }

    for handle in handles {
        if let Err(err) = handle.task.await {
            tracing::warn!(server = %handle.name, "Supervisor task panicked: {err:?}");
        }
    }
}

/// Send a [`SupervisorCommand::Restart`] to the named server.
///
/// Returns an error if no server with that name exists in `handles`.
///
/// In Phase 1, this is called from the foreground stdin command reader
/// (`handle_stdin_command` in `main.rs`) when the user types `restart <name>`.
pub async fn restart_server(handles: &[ServerHandle], name: &str) -> anyhow::Result<()> {
    let handle = handles
        .iter()
        .find(|h| h.name == name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found in config", name))?;

    handle
        .cmd_tx
        .send(SupervisorCommand::Restart)
        .await
        .with_context(|| {
            format!("Failed to send Restart to '{name}': supervisor task may have exited")
        })?;

    Ok(())
}
