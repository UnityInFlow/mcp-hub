mod cli;
mod config;
mod supervisor;
mod types;

use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use cli::{Cli, Commands};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Configure tracing verbosity based on -v / -vv flags.
    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        _ => tracing::Level::DEBUG,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Commands::Start => {
            let config = config::find_and_load_config(cli.config.as_deref())
                .context("Failed to load config")?;

            let shutdown = CancellationToken::new();

            // Spawn a supervisor task for every configured server.
            let mut handles = supervisor::start_all_servers(&config, shutdown.clone()).await;

            // Wait up to 10 s for all servers to leave their transient states.
            supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

            // Placeholder status table — Plan 03 adds formatted output (D-15).
            for handle in &handles {
                let (state, pid) = handle.state_rx.borrow().clone();
                println!(
                    "{:<20} {:<12} {}",
                    handle.name,
                    state.to_string(),
                    pid.map(|p| p.to_string())
                        .unwrap_or_else(|| "-".to_string())
                );
            }

            // Wait for Ctrl+C or SIGTERM, then trigger ordered shutdown (D-10).
            wait_for_shutdown_signal().await?;
            tracing::info!("Shutdown signal received — stopping all servers");
            shutdown.cancel();

            supervisor::stop_all_servers(handles).await;
        }

        Commands::Stop => {
            // Phase 1 foreground mode: nothing to stop from a separate invocation.
            // Daemon mode (Phase 3) will implement IPC-based stop.
            eprintln!("Stop via IPC is not implemented yet (Phase 3). Send Ctrl+C to the running mcp-hub process.");
        }

        Commands::Restart(args) => {
            let config = config::find_and_load_config(cli.config.as_deref())
                .context("Failed to load config")?;

            let shutdown = CancellationToken::new();
            let handles = supervisor::start_all_servers(&config, shutdown.clone()).await;

            supervisor::restart_server(&handles, &args.name)
                .await
                .with_context(|| format!("Failed to restart '{}'", args.name))?;

            println!("Restart signal sent to '{}'", args.name);

            wait_for_shutdown_signal().await?;
            shutdown.cancel();
            supervisor::stop_all_servers(handles).await;
        }
    }

    Ok(())
}

/// Wait for either Ctrl+C (SIGINT) or SIGTERM — whichever arrives first.
async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm =
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("Failed to listen for Ctrl+C")?;
            }
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("Failed to listen for Ctrl+C")?;
    }

    Ok(())
}
