mod cli;
mod config;
mod logs;
mod mcp;
mod output;
mod supervisor;
mod types;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use cli::{Cli, Commands};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    output::configure_tracing(cli.verbose);
    let color = output::use_colors(cli.no_color);

    match cli.command {
        Commands::Start { daemon: _ } => {
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

            // Print status table (D-15).
            let states = output::collect_states_from_handles(&handles);
            output::print_status_table(&states, color);

            // Run the interactive foreground loop (stdin commands + shutdown signal).
            run_foreground_loop(&handles, color, Arc::clone(&log_agg)).await?;

            tracing::info!("Shutting down all servers...");
            shutdown.cancel();
            supervisor::stop_all_servers(handles).await;

            tracing::info!("All servers stopped.");
            Ok(())
        }

        Commands::Stop => {
            // Phase 1: foreground mode only — stop is Ctrl+C.
            // Daemon mode (Phase 3) will implement socket-based stop.
            eprintln!("mcp-hub stop: no daemon running (foreground mode uses Ctrl+C to stop)");
            std::process::exit(1);
        }

        Commands::Restart(args) => {
            // Phase 1: restart requires the hub to be running in foreground.
            // The restart_server function IS implemented in supervisor.rs for Phase 3 wiring.
            eprintln!(
                "mcp-hub restart {}: restart is available during foreground operation via the supervisor. \
                 Daemon-mode restart will be available in a future version.",
                args.name
            );
            std::process::exit(1);
        }

        Commands::Status => {
            // Phase 2: requires daemon IPC (Phase 3).
            // In foreground mode, status is available via typing 'status' in the terminal.
            eprintln!(
                "mcp-hub status: no daemon running.\n\
                 In foreground mode, type 'status' in the running hub's terminal.\n\
                 Daemon-mode status will be available in a future version."
            );
            std::process::exit(1);
        }

        Commands::Logs(args) => {
            // Phase 2: requires daemon IPC (Phase 3) for separate process access.
            // In foreground mode, logs are visible in the terminal output directly.
            if args.follow {
                eprintln!(
                    "mcp-hub logs --follow: requires daemon mode.\n\
                     In foreground mode, server logs stream to the terminal automatically.\n\
                     Daemon-mode log streaming will be available in a future version."
                );
            } else {
                eprintln!(
                    "mcp-hub logs: no daemon running.\n\
                     In foreground mode, type 'logs' in the running hub's terminal to dump recent logs.\n\
                     Daemon-mode log access will be available in a future version."
                );
            }
            std::process::exit(1);
        }
    }
}

/// Run the interactive foreground loop.
///
/// Concurrently reads commands from stdin and waits for a shutdown signal
/// (Ctrl+C or SIGTERM). Typing `restart <name>` restarts the named server,
/// `status` reprints the status table, `logs` dumps recent logs, and `help`
/// lists all available commands.
///
/// When stdin is closed (e.g. piped input exhausted), the function falls back
/// to waiting for a shutdown signal so the hub does not exit unexpectedly.
async fn run_foreground_loop(
    handles: &[supervisor::ServerHandle],
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

        // Install the SIGTERM handler once outside the loop.
        let mut sigterm =
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(input)) => {
                            let trimmed = input.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            handle_stdin_command(trimmed, handles, color, &log_agg).await;
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
                            handle_stdin_command(trimmed, handles, color, &log_agg).await;
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
