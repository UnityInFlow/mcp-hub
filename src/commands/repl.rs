//! Interactive foreground command loop (`restart`, `status`, `logs`, `help`).

use std::sync::Arc;

use anyhow::Context as _;

use crate::{logs, output, supervisor};

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
pub async fn run_foreground_loop_shared(
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

    Ok(())
}
