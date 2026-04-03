mod cli;
mod config;
mod output;
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

    output::configure_tracing(cli.verbose);
    let color = output::use_colors(cli.no_color);

    match cli.command {
        Commands::Start => {
            let config = config::find_and_load_config(cli.config.as_deref())
                .context("Failed to load config")?;

            if config.servers.is_empty() {
                anyhow::bail!(
                    "No servers defined in config. Add servers to mcp-hub.toml or run `mcp-hub init`."
                );
            }

            let shutdown = CancellationToken::new();
            let mut handles = supervisor::start_all_servers(&config, shutdown.clone()).await;

            // Wait for servers to reach initial state (Running, Backoff, or Fatal).
            supervisor::wait_for_initial_states(&mut handles, Duration::from_secs(10)).await;

            // Print status table (D-15).
            let states = output::collect_states_from_handles(&handles);
            output::print_status_table(&states, color);

            // Block on Ctrl+C or SIGTERM (DMN-01).
            wait_for_shutdown_signal().await?;

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
    }
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
