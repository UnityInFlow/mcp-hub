//! Commands that talk to a running daemon over the control socket
//! (`stop`, `restart`, `status`, `logs`, `reload`, `gen-config --live`).

use anyhow::Context as _;

use crate::cli::{GenConfigArgs, GenConfigFormat, LogsArgs, RestartArgs};
use crate::control::{self, DaemonRequest, DaemonResponse};
use crate::{config, daemon, gen_config, output};

/// Send `request` to the daemon and turn a failure response into an error.
///
/// `failure_prefix` is prepended to the daemon's error message (`None` uses the
/// message as is). The error flows to `main`, which prints it and exits 1.
async fn send_checked(
    target: &daemon::Target,
    request: &DaemonRequest,
    timeout_secs: u64,
    failure_prefix: Option<&str>,
) -> anyhow::Result<DaemonResponse> {
    let response = control::send_daemon_command(&target.sock, request, timeout_secs).await?;
    if response.ok {
        return Ok(response);
    }
    let message = response
        .error
        .unwrap_or_else(|| "unknown error".to_string());
    match failure_prefix {
        Some(prefix) => anyhow::bail!("{prefix}: {message}"),
        None => anyhow::bail!("{message}"),
    }
}

/// `mcp-hub stop`: ask the daemon to shut down.
pub async fn stop(config: Option<&std::path::Path>) -> anyhow::Result<()> {
    let target = daemon::resolve_target(config)?;
    send_checked(&target, &DaemonRequest::Stop, 5, None).await?;
    println!("Daemon stop command sent. Shutting down...");
    Ok(())
}

/// `mcp-hub restart <name>`: restart one managed server.
pub async fn restart(config: Option<&std::path::Path>, args: RestartArgs) -> anyhow::Result<()> {
    let target = daemon::resolve_target(config)?;
    let server_name = args.name;
    send_checked(
        &target,
        &DaemonRequest::Restart {
            name: server_name.clone(),
        },
        10,
        None,
    )
    .await?;
    println!("Restart signal sent to '{server_name}'.");
    Ok(())
}

/// `mcp-hub status`: print the daemon's status as a table, or as JSON with `--json`.
pub async fn status(
    config: Option<&std::path::Path>,
    json: bool,
    color: bool,
) -> anyhow::Result<()> {
    let target = daemon::resolve_target(config)?;
    let response = send_checked(&target, &DaemonRequest::Status, 5, Some("Status failed")).await?;
    let Some(data) = response.data else {
        return Ok(());
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        // The JSON stays the daemon's raw response (`gen-config --live` and
        // scripts parse it); the human view gets a header naming the daemon.
        if let Some(meta) = &target.meta {
            println!(
                "Daemon for {} (pid {}, web UI http://127.0.0.1:{})",
                meta.config_path, meta.pid, meta.web_port
            );
        }
        println!("{}", output::format_status_json(&data, color)?);
    }
    Ok(())
}

/// `mcp-hub logs`: print recent log lines from the daemon.
pub async fn logs(config: Option<&std::path::Path>, args: LogsArgs) -> anyhow::Result<()> {
    let target = daemon::resolve_target(config)?;
    if args.follow {
        return logs_follow(&target, args).await;
    }
    let request = DaemonRequest::Logs {
        server: args.server,
        lines: args.lines,
    };
    let response = send_checked(&target, &request, 5, Some("Logs failed")).await?;
    if let Some(lines) = response.data.as_ref().and_then(|d| d.as_array()) {
        for line in lines {
            if let Some(s) = line.as_str() {
                println!("{s}");
            }
        }
    }
    Ok(())
}

/// `mcp-hub logs --follow`: print the recent lines, then stream live ones until
/// the daemon closes the connection or the user presses Ctrl+C.
async fn logs_follow(target: &daemon::Target, args: LogsArgs) -> anyhow::Result<()> {
    use std::io::Write as _;

    let request = DaemonRequest::LogsFollow {
        server: args.server,
        lines: args.lines,
    };
    let mut stream = control::open_daemon_stream(&target.sock, &request, 5)
        .await
        .map_err(|e| anyhow::anyhow!("Logs failed: {e}"))?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            event = stream.next_event() => match event? {
                Some(event) => {
                    let mut out = std::io::stdout().lock();
                    // A closed pipe (e.g. `| head`) ends the follow quietly.
                    if writeln!(out, "{}", event.render()).and_then(|()| out.flush()).is_err() {
                        return Ok(());
                    }
                }
                None => return Ok(()),
            },
        }
    }
}

/// `mcp-hub reload`: ask the daemon to re-read its config and apply the diff.
pub async fn reload(config: Option<&std::path::Path>) -> anyhow::Result<()> {
    let target = daemon::resolve_target(config)?;
    // Replacing servers can take up to 5 s each to shut down.
    let response = send_checked(&target, &DaemonRequest::Reload, 30, Some("Reload failed")).await?;
    match response.data {
        Some(data) => {
            let summary: control::ReloadSummary = serde_json::from_value(data)
                .context("Daemon returned an invalid reload summary")?;
            println!("Reload complete: {summary}.");
        }
        None => println!("Reload complete."),
    }
    Ok(())
}

/// `mcp-hub gen-config`: render a Claude Code / Cursor / Codex / Gemini MCP config to stdout.
pub async fn gen_config(
    config_path: Option<&std::path::Path>,
    args: GenConfigArgs,
) -> anyhow::Result<()> {
    let (config, secret_vars) =
        config::find_and_load_config_report(config_path).context("Failed to load config")?;

    if config.servers.is_empty() {
        eprintln!("\u{26a0} No servers configured in mcp-hub.toml");
    }

    if !secret_vars.is_empty() {
        eprintln!(
            "warning: gen-config is writing secret values expanded from {} to stdout; \
             do not commit or share the output",
            secret_vars.join(", ")
        );
    }

    let live_info: Option<Vec<gen_config::ServerLiveInfo>> = if args.live {
        let target = daemon::resolve_target(config_path)?;
        let response =
            control::send_daemon_command(&target.sock, &DaemonRequest::Status, 5).await?;
        Some(gen_config::parse_live_info(&response)?)
    } else {
        None
    };

    let rendered = match args.format {
        GenConfigFormat::Claude => gen_config::render_claude_config(&config, live_info.as_deref())?,
        GenConfigFormat::Cursor => gen_config::render_cursor_config(&config, live_info.as_deref())?,
        GenConfigFormat::Codex => gen_config::render_codex_config(&config, live_info.as_deref())?,
        GenConfigFormat::Gemini => gen_config::render_gemini_config(&config, live_info.as_deref())?,
    };

    // Annotations go to stderr; stdout is the bare config document (JSON,
    // or TOML for Codex) so it can be redirected straight into the client's config.
    for note in &rendered.notes {
        eprintln!("{note}");
    }
    print!("{}", rendered.json);
    Ok(())
}
