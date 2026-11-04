use clap::Parser;
use mcp_hub::cli::{Cli, Commands};
use mcp_hub::commands::start::DaemonStartup;
use mcp_hub::commands::{client, start};
use mcp_hub::{daemon, init, output};

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
        let config = start::load_start_config(cli.config.as_deref())?;
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
                return start::run_daemon(cli.config.as_deref(), startup).await;
            }
            start::run_foreground(cli.config.as_deref(), color).await
        }
        Commands::Stop => client::stop().await,
        Commands::Restart(args) => client::restart(args).await,
        Commands::Status => client::status().await,
        Commands::Logs(args) => client::logs(args).await,
        Commands::Reload => client::reload().await,
        Commands::Init => init::run_init_wizard(),
        Commands::GenConfig(args) => client::gen_config(cli.config.as_deref(), args).await,
    }
}
