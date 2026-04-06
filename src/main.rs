mod cli;
mod config;
mod control;
mod daemon;
mod logs;
mod mcp;
mod output;
mod supervisor;
mod types;
mod web;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use cli::{Cli, Commands};
use tokio_util::sync::CancellationToken;

/// Synchronous entry point.
///
/// Daemonization via `fork(2)` **must** happen before the Tokio runtime is
/// created — forking after threads are spawned leads to undefined behaviour.
/// This function handles the pre-fork work (duplicate-daemon check, fork,
/// PID file), then builds the Tokio runtime and hands off to `async_main`.
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Detect whether `--daemon` was requested so we can fork before Tokio starts.
    let is_daemon = matches!(cli.command, Commands::Start { daemon: true });

    #[cfg(unix)]
    if is_daemon {
        let sock = daemon::socket_path()?;
        let pid = daemon::pid_path()?;
        // If a live daemon is already running, bail out here (before fork).
        daemon::check_existing_daemon(&sock, &pid)?;
        // Fork into background — the parent process exits here.
        daemon::daemonize_process()?;
        // We are now the daemon child. Write our PID.
        daemon::write_pid_file(&pid)?;
    }

    #[cfg(not(unix))]
    if is_daemon {
        anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.");
    }

    // Build the Tokio runtime *after* the fork so threads are not duplicated.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main(cli))
}

/// Async main — all Tokio-dependent code lives here.
async fn async_main(cli: Cli) -> anyhow::Result<()> {
    output::configure_tracing(cli.verbose);
    let color = output::use_colors(cli.no_color);

    match cli.command {
        Commands::Start { daemon } => {
            let config = config::find_and_load_config(cli.config.as_deref())
                .context("Failed to load config")?;

            if config.servers.is_empty() {
                anyhow::bail!(
                    "No servers defined in config. Add servers to mcp-hub.toml or run `mcp-hub init`."
                );
            }

            let shutdown = CancellationToken::new();

            let server_names: Vec<String> = config.servers.keys().cloned().collect();
            let log_agg = Arc::new(logs::LogAggregator::new(&server_names, 10_000));

            let mut handles =
                supervisor::start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg))
                    .await;

            // Wait for servers to reach initial state (Running, Backoff, or Fatal).
            supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

            if daemon {
                // ── Daemon mode ─────────────────────────────────────────────
                // No TTY in daemon mode — run the control socket instead of the
                // foreground stdin loop.
                let sock = daemon::socket_path()?;
                let pid = daemon::pid_path()?;

                let daemon_state = Arc::new(control::DaemonState {
                    handles: Arc::new(tokio::sync::Mutex::new(handles)),
                    log_agg: Arc::clone(&log_agg),
                    shutdown: shutdown.clone(),
                    color: false,
                });

                // Spawn web UI server (Phase 4 -- WEB-01).
                let web_port = config.hub.web_port;
                let web_state = Arc::new(web::WebState {
                    handles: Arc::clone(&daemon_state.handles),
                    log_agg: Arc::clone(&log_agg),
                });
                let web_shutdown = shutdown.clone();
                let web_task = tokio::spawn(async move {
                    if let Err(e) = web::start_web_server(web_port, web_state, web_shutdown).await {
                        tracing::error!("Web UI error: {e}");
                    }
                });
                tracing::info!("Web UI available at http://127.0.0.1:{web_port}");

                // Spawn the control socket listener as a background task.
                let socket_task = {
                    let state = Arc::clone(&daemon_state);
                    let sock_clone = sock.clone();
                    tokio::spawn(async move {
                        if let Err(e) = control::run_control_socket(&sock_clone, state).await {
                            tracing::error!("Control socket error: {e}");
                        }
                    })
                };

                // Track the current config so reloads can diff against it.
                let mut current_config = config;

                // Block until a shutdown signal (SIGTERM, Ctrl+C) or a Stop
                // command via the control socket cancels the token.
                // SIGHUP triggers a config reload without stopping the daemon.
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};

                    let mut sighup =
                        signal(SignalKind::hangup()).context("Failed to install SIGHUP handler")?;
                    let mut sigterm = signal(SignalKind::terminate())
                        .context("Failed to install SIGTERM handler")?;

                    loop {
                        tokio::select! {
                            _ = sighup.recv() => {
                                tracing::info!("SIGHUP received — reloading config");
                                handle_reload(
                                    &daemon_state,
                                    cli.config.as_deref(),
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
                                result.context("Failed to listen for Ctrl+C")?;
                                tracing::info!("Ctrl+C received — shutting down daemon");
                                break;
                            }
                            _ = shutdown.cancelled() => {
                                // Stop command received via control socket.
                                break;
                            }
                        }
                    }
                }

                #[cfg(not(unix))]
                {
                    tokio::select! {
                        result = tokio::signal::ctrl_c() => {
                            result.context("Failed to listen for Ctrl+C")?;
                            tracing::info!("Ctrl+C received — shutting down daemon");
                        }
                        _ = shutdown.cancelled() => {
                            // Stop command received via control socket.
                        }
                    }
                }

                tracing::info!("Shutting down daemon...");
                shutdown.cancel();

                // Wait for the control socket to finish serving in-flight requests.
                socket_task.await.ok();
                web_task.abort(); // Stop web server

                // Stop all managed servers.
                // Clone the handles Arc so we can try_unwrap it once daemon_state is dropped.
                let handles_arc = Arc::clone(&daemon_state.handles);
                drop(daemon_state);
                let final_handles = Arc::try_unwrap(handles_arc)
                    .map_err(|_| {
                        anyhow::anyhow!("Cannot unwrap daemon handles — Arc still shared")
                    })?
                    .into_inner();
                supervisor::stop_all_servers(final_handles).await;

                // Remove the PID file now that the daemon is fully stopped.
                daemon::remove_pid_file(&pid);

                tracing::info!("Daemon stopped.");
            } else {
                // ── Foreground mode ─────────────────────────────────────────
                // Print initial status, then wrap handles in Arc<Mutex> for web server sharing.
                let states = output::collect_states_from_handles(&handles);
                output::print_status_table(&states, color);

                // Wrap handles for web server sharing.
                let handles_arc = Arc::new(tokio::sync::Mutex::new(handles));

                // Spawn web UI server (Phase 4 -- WEB-01).
                let web_state = Arc::new(web::WebState {
                    handles: Arc::clone(&handles_arc),
                    log_agg: Arc::clone(&log_agg),
                });
                let web_shutdown = shutdown.clone();
                let _web_task = tokio::spawn(async move {
                    if let Err(e) =
                        web::start_web_server(config.hub.web_port, web_state, web_shutdown).await
                    {
                        tracing::error!("Web UI error: {e}");
                    }
                });
                tracing::info!(
                    "Web UI available at http://127.0.0.1:{}",
                    config.hub.web_port
                );

                run_foreground_loop_shared(&handles_arc, color, Arc::clone(&log_agg)).await?;

                tracing::info!("Shutting down all servers...");
                shutdown.cancel();

                // Recover handles for stop.
                let final_handles = Arc::try_unwrap(handles_arc)
                    .map_err(|_| {
                        anyhow::anyhow!("Cannot unwrap foreground handles — Arc still shared")
                    })?
                    .into_inner();
                supervisor::stop_all_servers(final_handles).await;
                tracing::info!("All servers stopped.");
            }

            Ok(())
        }

        Commands::Stop => {
            // Daemon mode stop — connect to the daemon and send Stop.
            let sock = daemon::socket_path()?;
            let response =
                control::send_daemon_command(&sock, &control::DaemonRequest::Stop, 5).await?;
            if response.ok {
                println!("Daemon stop command sent. Shutting down...");
            } else {
                eprintln!(
                    "Error: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Restart(args) => {
            // Daemon mode restart — connect to the daemon and send Restart.
            let sock = daemon::socket_path()?;
            let server_name = args.name;
            let response = control::send_daemon_command(
                &sock,
                &control::DaemonRequest::Restart {
                    name: server_name.clone(),
                },
                10,
            )
            .await?;
            if response.ok {
                println!("Restart signal sent to '{server_name}'.");
            } else {
                eprintln!(
                    "Error: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Status => {
            // Daemon mode status — connect to the daemon and request status.
            let sock = daemon::socket_path()?;
            let request = control::DaemonRequest::Status;
            let response = control::send_daemon_command(&sock, &request, 5).await?;
            if response.ok {
                if let Some(data) = response.data {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                }
            } else {
                eprintln!(
                    "Status failed: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Logs(args) => {
            // Daemon mode logs — connect to the daemon and request logs.
            let sock = daemon::socket_path()?;
            let request = control::DaemonRequest::Logs {
                server: args.server,
                lines: args.lines,
            };
            let response = control::send_daemon_command(&sock, &request, 5).await?;
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
                eprintln!(
                    "Logs failed: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Reload => {
            // Send a Reload request to the running daemon via the control socket.
            // The daemon receives this, sends SIGHUP to itself, and the main event
            // loop picks it up and calls handle_reload.
            let sock = daemon::socket_path()?;
            let response =
                control::send_daemon_command(&sock, &control::DaemonRequest::Reload, 5).await?;
            if response.ok {
                println!("Reload signal sent to daemon.");
            } else {
                eprintln!(
                    "Reload failed: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// Reload configuration from disk and apply changes to the running daemon.
///
/// Diffs the new config against `current_config` and starts/stops/restarts
/// servers as needed. Updates `current_config` on success so subsequent reloads
/// diff against the latest state.
async fn handle_reload(
    state: &Arc<control::DaemonState>,
    config_path: Option<&std::path::Path>,
    current_config: &mut config::HubConfig,
    shutdown: &CancellationToken,
    log_agg: &Arc<logs::LogAggregator>,
) {
    match config::find_and_load_config(config_path) {
        Ok(new_config) => {
            let mut handles = state.handles.lock().await;
            let (added, removed, changed) = supervisor::apply_config_diff(
                &mut handles,
                current_config,
                &new_config,
                shutdown,
                log_agg,
            )
            .await;
            tracing::info!(added, removed, changed, "Config reload complete");
            *current_config = new_config;
        }
        Err(e) => {
            tracing::error!("Config reload failed: {e}");
        }
    }
}

/// Run the interactive foreground loop with Arc-wrapped handles for web server sharing.
///
/// Concurrently reads commands from stdin and waits for a shutdown signal
/// (Ctrl+C or SIGTERM). Typing `restart <name>` restarts the named server,
/// `status` reprints the status table, `logs` dumps recent logs, and `help`
/// lists all available commands.
///
/// Handles are wrapped in `Arc<Mutex<Vec<ServerHandle>>>` so they can be shared
/// concurrently with the web server spawned in foreground mode.
///
/// When stdin is closed (e.g. piped input exhausted), the function falls back
/// to waiting for a shutdown signal so the hub does not exit unexpectedly.
async fn run_foreground_loop_shared(
    handles: &Arc<tokio::sync::Mutex<Vec<supervisor::ServerHandle>>>,
    color: bool,
    log_agg: Arc<logs::LogAggregator>,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    // Print a hint that commands are available.
    eprintln!("Type 'help' for available commands, or press Ctrl+C to stop.");

    // Run separate implementations per platform so that `tokio::select!` branches
    // are uniform (cfg attributes are not supported inside select! arms).
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        // Install the SIGTERM and SIGHUP handlers once outside the loop.
        let mut sigterm =
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;
        let mut sighup =
            signal(SignalKind::hangup()).context("Failed to install SIGHUP handler")?;

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(input)) => {
                            let trimmed = input.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let h = handles.lock().await;
                            handle_stdin_command(trimmed, &h, color, &log_agg).await;
                        }
                        Ok(None) => {
                            // stdin closed — fall back to waiting for a signal.
                            tracing::debug!("stdin closed; waiting for shutdown signal");
                            wait_for_shutdown_signal().await?;
                            break;
                        }
                        Err(err) => {
                            tracing::warn!("Error reading stdin: {err}");
                            wait_for_shutdown_signal().await?;
                            break;
                        }
                    }
                }
                result = tokio::signal::ctrl_c() => {
                    result.context("Failed to listen for Ctrl+C")?;
                    tracing::info!("Ctrl+C received");
                    break;
                }
                _ = sigterm.recv() => {
                    tracing::info!("SIGTERM received");
                    break;
                }
                _ = sighup.recv() => {
                    // Config reload in foreground mode — log a notice. Full reload
                    // requires access to handles and config path which are not
                    // available here; use `mcp-hub reload` against a daemon instead.
                    tracing::info!("SIGHUP received in foreground mode (use daemon mode for config reload)");
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(input)) => {
                            let trimmed = input.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let h = handles.lock().await;
                            handle_stdin_command(trimmed, &h, color, &log_agg).await;
                        }
                        Ok(None) => {
                            tracing::debug!("stdin closed; waiting for shutdown signal");
                            wait_for_shutdown_signal().await?;
                            break;
                        }
                        Err(err) => {
                            tracing::warn!("Error reading stdin: {err}");
                            wait_for_shutdown_signal().await?;
                            break;
                        }
                    }
                }
                result = tokio::signal::ctrl_c() => {
                    result.context("Failed to listen for Ctrl+C")?;
                    tracing::info!("Ctrl+C received");
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Dispatch a single line of stdin input to the appropriate handler.
async fn handle_stdin_command(
    input: &str,
    handles: &[supervisor::ServerHandle],
    color: bool,
    log_agg: &logs::LogAggregator,
) {
    if let Some(name) = input.strip_prefix("restart ") {
        let name = name.trim();
        if name.is_empty() {
            eprintln!("Usage: restart <server-name>");
            return;
        }
        match supervisor::restart_server(handles, name).await {
            Ok(()) => {
                eprintln!("Restart signal sent to '{name}'. Waiting for new state...");
                // Give the supervisor time to stop and re-spawn the process.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let states = output::collect_states_from_handles(handles);
                output::print_status_table(&states, color);
            }
            Err(err) => {
                eprintln!("Error: {err}");
            }
        }
    } else if input == "status" {
        let states = output::collect_states_from_handles(handles);
        output::print_status_table(&states, color);
    } else if input == "logs" || input.starts_with("logs ") {
        // "logs"         -> dump last 100 lines from all servers
        // "logs <name>"  -> dump last 100 lines for specific server
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        if parts.len() == 1 {
            // All servers — merge and show last 100.
            let lines = log_agg.snapshot_all().await;
            let tail = if lines.len() > 100 {
                &lines[lines.len() - 100..]
            } else {
                &lines[..]
            };
            for line in tail {
                println!("{}", logs::format_log_line(line, color));
            }
            if lines.is_empty() {
                eprintln!("No logs captured yet.");
            }
        } else {
            let server_name = parts[1].trim();
            match log_agg.get_buffer(server_name) {
                Some(buf) => {
                    let lines = buf.snapshot_last(100).await;
                    for line in &lines {
                        println!("{}", logs::format_log_line(line, color));
                    }
                    if lines.is_empty() {
                        eprintln!("No logs captured for '{server_name}' yet.");
                    }
                }
                None => {
                    eprintln!("Unknown server: '{server_name}'");
                }
            }
        }
    } else if input == "help" {
        eprintln!("Available commands:");
        eprintln!("  restart <name>  — Restart the named server");
        eprintln!("  status          — Show current server status");
        eprintln!("  logs            - Show recent logs from all servers");
        eprintln!("  logs <name>     - Show recent logs for a specific server");
        eprintln!("  help            — Show this help message");
        eprintln!("  Ctrl+C          — Shut down all servers and exit");
    } else {
        eprintln!("Unknown command: '{input}'. Type 'help' for available commands.");
    }
}

/// Wait for either Ctrl+C (SIGINT) or SIGTERM — whichever arrives first.
///
/// Used as a fallback when stdin is closed or encounters an error.
async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm =
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("Failed to listen for Ctrl+C")?;
                tracing::info!("Ctrl+C received");
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("Failed to install Ctrl+C handler")?;
        tracing::info!("Ctrl+C received");
    }

    Ok(())
}
