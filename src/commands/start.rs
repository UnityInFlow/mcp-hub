//! `mcp-hub start`: foreground supervisor and the background daemon.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use tokio_util::sync::CancellationToken;

use crate::{config, control, daemon, logs, output, supervisor, web};

use super::repl;

/// State handed from the pre-fork phase of `main` to the daemon.
pub struct DaemonStartup {
    /// Config loaded and validated *before* forking, so errors reach the terminal.
    pub config: config::HubConfig,
    /// Reports readiness (or the startup error) to the waiting foreground parent.
    pub notifier: daemon::ReadyNotifier,
    /// Control socket path.
    pub sock: std::path::PathBuf,
    /// PID file path (written before the runtime starts, removed on every exit).
    pub pid: std::path::PathBuf,
    /// Exclusive lock on the PID file; held until the daemon has removed it.
    pub pid_lock: daemon::PidFileLock,
}

/// Load the config for `start` and reject an empty server list.
pub fn load_start_config(
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<config::HubConfig> {
    let config = config::find_and_load_config(config_path).context("Failed to load config")?;

    if config.servers.is_empty() {
        anyhow::bail!(
            "No servers defined in config. Add servers to mcp-hub.toml or run `mcp-hub init`."
        );
    }

    Ok(config)
}
/// Run the hub in the foreground: start every server, serve the web UI and
/// the interactive stdin loop until a shutdown signal arrives.
pub async fn run_foreground(
    config_path: Option<&std::path::Path>,
    color: bool,
) -> anyhow::Result<()> {
    let config = load_start_config(config_path)?;

    let shutdown = CancellationToken::new();

    let server_names: Vec<String> = config.servers.keys().cloned().collect();
    let log_agg = Arc::new(logs::LogAggregator::new(&server_names, 10_000));

    let mut handles =
        supervisor::start_all_servers(&config, shutdown.clone(), Arc::clone(&log_agg)).await;

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
            port: config.hub.web_port,
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

        repl::run_foreground_loop_shared(&handles_arc, color, Arc::clone(&log_agg)).await?;

        tracing::info!("Shutting down all servers...");
        shutdown.cancel();

        // Abort the web task and await it so it releases its Arc clone before try_unwrap.
        web_task.abort();
        let _ = web_task.await;

        // Recover handles for stop.
        let final_handles = Arc::try_unwrap(handles_arc)
            .map_err(|_| anyhow::anyhow!("Cannot unwrap foreground handles — Arc still shared"))?
            .into_inner();
        supervisor::stop_all_servers(final_handles).await;
        tracing::info!("All servers stopped.");
    }

    Ok(())
}

/// Run the daemon: start servers, bind the sockets, tell the parent we are
/// ready, then serve until shut down.
///
/// Every error path reports to the waiting parent (if it has not been told yet)
/// and removes the PID file, so a failed start never leaves stale state.
pub async fn run_daemon(
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
        config_path: config_path.map(std::path::Path::to_path_buf),
        // Reloads diff against this, so it must be what the servers started from.
        current_config: tokio::sync::Mutex::new(config),
        views: Arc::clone(&server_views),
    });

    // Spawn web UI server (Phase 4 -- WEB-01).
    let web_state = Arc::new(web::WebState {
        servers: Arc::clone(&server_views),
        log_agg: Arc::clone(&log_agg),
        port: web_port,
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
