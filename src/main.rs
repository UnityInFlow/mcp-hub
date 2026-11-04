mod cli;
mod config;
mod control;
mod daemon;
mod gen_config;
mod init;
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

/// State handed from the pre-fork phase of `main` to the daemon.
struct DaemonStartup {
    /// Config loaded and validated *before* forking, so errors reach the terminal.
    config: config::HubConfig,
    /// Reports readiness (or the startup error) to the waiting foreground parent.
    notifier: daemon::ReadyNotifier,
    /// Control socket path.
    sock: std::path::PathBuf,
    /// PID file path (written before the runtime starts, removed on every exit).
    pid: std::path::PathBuf,
    /// Exclusive lock on the PID file; held until the daemon has removed it.
    pid_lock: daemon::PidFileLock,
}

/// Load the config for `start` and reject an empty server list.
fn load_start_config(config_path: Option<&std::path::Path>) -> anyhow::Result<config::HubConfig> {
    let config = config::find_and_load_config(config_path).context("Failed to load config")?;

    if config.servers.is_empty() {
        anyhow::bail!(
            "No servers defined in config. Add servers to mcp-hub.toml or run `mcp-hub init`."
        );
    }

    Ok(config)
}

/// Synchronous entry point.
///
/// Daemonization via `fork(2)` **must** happen before the Tokio runtime is
/// created — forking after threads are spawned leads to undefined behaviour.
/// This function handles the pre-fork work (duplicate-daemon check, config
/// load and validation, fork, PID file), then builds the Tokio runtime and
/// hands off to `async_main`.
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Detect whether `--daemon` was requested so we can fork before Tokio starts.
    let is_daemon = matches!(cli.command, Commands::Start { daemon: true, .. });

    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut startup: Option<DaemonStartup> = None;

    #[cfg(unix)]
    if is_daemon {
        let log_file = if let Commands::Start { log_file, .. } = &cli.command {
            log_file.clone()
        } else {
            None
        };
        let log_path = daemon::log_path(log_file.as_deref())?;
        let sock = daemon::socket_path()?;
        let pid = daemon::pid_path()?;
        // If a live daemon is already running, bail out here (before fork).
        daemon::check_existing_daemon(&sock, &pid)?;
        // Load and validate the config while we still have a terminal, so a bad
        // config fails the command instead of vanishing into the background.
        let config = load_start_config(cli.config.as_deref())?;
        // Fork into background — the original process waits for readiness and
        // exits inside this call; only the daemon returns.
        let notifier = daemon::daemonize_process(&log_path)?;
        // We are now the daemon child. Write our PID and lock the file; losing
        // the lock means another daemon started in the meantime.
        let pid_lock = match daemon::write_pid_file(&pid) {
            Ok(lock) => lock,
            Err(e) => {
                notifier.fail(&format!("{e:#}"));
                return Err(e);
            }
        };
        startup = Some(DaemonStartup {
            config,
            notifier,
            sock,
            pid,
            pid_lock,
        });
    }

    #[cfg(not(unix))]
    if is_daemon {
        anyhow::bail!("Daemon mode is not supported on Windows. Use foreground mode instead.");
    }

    // Build the Tokio runtime *after* the fork so threads are not duplicated.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            if let Some(s) = startup {
                daemon::remove_pid_file(&s.pid);
                s.notifier
                    .fail(&format!("Failed to start async runtime: {e}"));
            }
            return Err(e.into());
        }
    };
    runtime.block_on(async_main(cli, startup))
}

/// Run the daemon: start servers, bind the sockets, tell the parent we are
/// ready, then serve until shut down.
///
/// Every error path reports to the waiting parent (if it has not been told yet)
/// and removes the PID file, so a failed start never leaves stale state.
async fn run_daemon(
    config_path: Option<&std::path::Path>,
    startup: DaemonStartup,
) -> anyhow::Result<()> {
    let DaemonStartup {
        config,
        notifier,
        sock,
        pid,
        pid_lock,
    } = startup;
    let mut notifier = Some(notifier);

    let result = run_daemon_inner(config_path, config, &sock, &mut notifier).await;

    if let Err(e) = &result {
        if let Some(n) = notifier.take() {
            n.fail(&format!("{e:#}"));
        }
    }
    // Success or failure, the daemon is gone after this point. We hold the PID
    // file lock, so no other daemon owns these files; unlink before releasing.
    let _ = std::fs::remove_file(&sock);
    daemon::remove_pid_file(&pid);
    drop(pid_lock);
    result
}

/// Bind the web UI and control socket listeners (web first, so a web bind
/// failure leaves no socket file behind).
async fn bind_daemon_listeners(
    sock: &std::path::Path,
    web_port: u16,
) -> anyhow::Result<(tokio::net::TcpListener, control::ControlListener)> {
    let web_listener = web::bind_web_listener(web_port).await?;
    let control_listener = control::bind_control_socket(sock).await?;
    Ok((web_listener, control_listener))
}

async fn run_daemon_inner(
    config_path: Option<&std::path::Path>,
    config: config::HubConfig,
    sock: &std::path::Path,
    notifier: &mut Option<daemon::ReadyNotifier>,
) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();

    // Install signal handlers up front so every later step can use `?`
    // without stranding already-started servers over a handler failure.
    #[cfg(unix)]
    let (mut sighup, mut sigterm) = {
        use tokio::signal::unix::{signal, SignalKind};
        (
            signal(SignalKind::hangup()).context("Failed to install SIGHUP handler")?,
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?,
        )
    };

    let server_names: Vec<String> = config.servers.keys().cloned().collect();
    let log_agg = Arc::new(logs::LogAggregator::new(&server_names, 10_000));

    let mut handles =
        supervisor::start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

    // Wait for servers to reach initial state (Running, Backoff, or Fatal).
    supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

    // Bind everything that can fail *before* declaring the daemon ready; on
    // failure stop the servers we just started.
    let web_port = config.hub.web_port;
    let (web_listener, control_listener) = match bind_daemon_listeners(sock, web_port).await {
        Ok(listeners) => listeners,
        Err(e) => {
            shutdown.cancel();
            supervisor::stop_all_servers(handles).await;
            return Err(e);
        }
    };

    // Web UI reads its own snapshot views so it never contends on
    // the handles mutex that config reload holds across shutdowns.
    let server_views = web::new_server_views(&handles);
    let daemon_state = Arc::new(control::DaemonState {
        handles: Arc::new(tokio::sync::Mutex::new(handles)),
        log_agg: Arc::clone(&log_agg),
        shutdown: shutdown.clone(),
        color: false,
        config_path: config_path.map(std::path::Path::to_path_buf),
        // Reloads diff against this, so it must be what the servers started from.
        current_config: tokio::sync::Mutex::new(config),
        views: Arc::clone(&server_views),
    });

    // Spawn web UI server (Phase 4 -- WEB-01).
    let web_state = Arc::new(web::WebState {
        servers: Arc::clone(&server_views),
        log_agg: Arc::clone(&log_agg),
    });
    let web_shutdown = shutdown.clone();
    let web_task = tokio::spawn(async move {
        if let Err(e) = web::serve_web(web_listener, web_state, web_shutdown).await {
            tracing::error!("Web UI error: {e}");
        }
    });
    tracing::info!("Web UI available at http://127.0.0.1:{web_port}");

    // Spawn the control socket listener as a background task.
    let socket_task = {
        let state = Arc::clone(&daemon_state);
        let sock_owned = sock.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) =
                control::serve_control_socket(control_listener, &sock_owned, state).await
            {
                tracing::error!("Control socket error: {e}");
            }
        })
    };

    // Everything that can fail at startup has succeeded: release the parent.
    if let Some(n) = notifier.take() {
        n.ready();
    }

    // Block until a shutdown signal (SIGTERM, Ctrl+C) or a Stop
    // command via the control socket cancels the token.
    // SIGHUP triggers a config reload without stopping the daemon.
    #[cfg(unix)]
    loop {
        tokio::select! {
            _ = sighup.recv() => {
                tracing::info!("SIGHUP received — reloading config");
                if let Err(e) = control::reload_config(&daemon_state).await {
                    tracing::error!("Config reload failed: {e:#}");
                }
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
    let _ = web_task.await;

    // Stop all managed servers. Take the handles out from under the lock rather
    // than unwrapping the Arc: a connection handler (e.g. a client that never
    // sent a request) may still hold a `DaemonState` clone, and that must not
    // stop us from stopping the servers.
    let final_handles = std::mem::take(&mut *daemon_state.handles.lock().await);
    supervisor::stop_all_servers(final_handles).await;

    tracing::info!("Daemon stopped.");
    Ok(())
}

/// Async main — all Tokio-dependent code lives here.
async fn async_main(cli: Cli, startup: Option<DaemonStartup>) -> anyhow::Result<()> {
    // A daemon has no terminal to be quiet for: its log file is the only record
    // of bind failures, reloads, backoff and Fatal transitions, so log at least
    // `info` there.
    let verbose = if startup.is_some() {
        cli.verbose.max(1)
    } else {
        cli.verbose
    };
    output::configure_tracing(verbose);
    let color = output::use_colors(cli.no_color);

    match cli.command {
        Commands::Start { daemon, .. } => {
            if daemon {
                let startup = startup
                    .ok_or_else(|| anyhow::anyhow!("Daemon startup state missing after fork"))?;
                return run_daemon(cli.config.as_deref(), startup).await;
            }

            let config = load_start_config(cli.config.as_deref())?;

            let shutdown = CancellationToken::new();

            let server_names: Vec<String> = config.servers.keys().cloned().collect();
            let log_agg = Arc::new(logs::LogAggregator::new(&server_names, 10_000));

            let mut handles =
                supervisor::start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg))
                    .await;

            // Wait for servers to reach initial state (Running, Backoff, or Fatal).
            supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

            // ── Foreground mode ─────────────────────────────────────────────
            {
                // Print initial status, then wrap handles in Arc<Mutex> for web server sharing.
                let states = output::collect_states_from_handles(&handles);
                output::print_status_table(&states, color);

                // Foreground mode has no reload, so the views never change.
                let server_views = web::new_server_views(&handles);

                // Wrap handles for the foreground loop (shared Arc<Mutex>).
                let handles_arc = Arc::new(tokio::sync::Mutex::new(handles));

                // Spawn web UI server (Phase 4 -- WEB-01).
                let web_state = Arc::new(web::WebState {
                    servers: server_views,
                    log_agg: Arc::clone(&log_agg),
                });
                let web_shutdown = shutdown.clone();
                let web_task = tokio::spawn(async move {
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

                // Abort the web task and await it so it releases its Arc clone before try_unwrap.
                web_task.abort();
                let _ = web_task.await;

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
            // Ask the running daemon to reload; it applies the new config
            // itself and reports what changed (or why it could not).
            let sock = daemon::socket_path()?;
            // Replacing servers can take up to 5 s each to shut down.
            let response =
                control::send_daemon_command(&sock, &control::DaemonRequest::Reload, 30).await?;
            if response.ok {
                match response.data {
                    Some(data) => {
                        let summary: control::ReloadSummary = serde_json::from_value(data)
                            .context("Daemon returned an invalid reload summary")?;
                        println!("Reload complete: {summary}.");
                    }
                    None => println!("Reload complete."),
                }
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

        Commands::Init => init::run_init_wizard(),

        Commands::GenConfig(args) => {
            let config = config::find_and_load_config(cli.config.as_deref())
                .context("Failed to load config")?;

            if config.servers.is_empty() {
                eprintln!("\u{26a0} No servers configured in mcp-hub.toml");
            }

            let live_info: Option<Vec<gen_config::ServerLiveInfo>> = if args.live {
                let sock = daemon::socket_path()?;
                let response =
                    control::send_daemon_command(&sock, &control::DaemonRequest::Status, 5).await?;
                Some(gen_config::parse_live_info(&response)?)
            } else {
                None
            };

            let rendered = match args.format.as_str() {
                "claude" => gen_config::render_claude_config(&config, live_info.as_deref())?,
                "cursor" => gen_config::render_cursor_config(&config, live_info.as_deref())?,
                other => anyhow::bail!(
                    "Unknown format '{other}'. Use --format claude or --format cursor."
                ),
            };

            // Annotations go to stderr; stdout is the bare JSON document so it
            // can be redirected straight into .mcp.json / Cursor's config.
            for note in &rendered.notes {
                eprintln!("{note}");
            }
            print!("{}", rendered.json);
            Ok(())
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
