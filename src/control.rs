/// Unix domain socket IPC for daemon mode.
///
/// Defines the `DaemonRequest`/`DaemonResponse` wire protocol (newline-delimited
/// JSON over a Unix domain socket), the server-side listener, and the client
/// helper used by CLI commands that connect to a running daemon.
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::logs::LogAggregator;
use crate::supervisor::ServerHandle;

// ─────────────────────────────────────────────────────────────────────────────
// IPC message types
// ─────────────────────────────────────────────────────────────────────────────

/// A request sent to the daemon via the control socket.
///
/// Serialised as a tagged JSON object: `{"cmd": "<variant>", ...fields}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// Return the current status of all managed servers.
    Status,
    /// Stop all servers and shut the daemon down.
    Stop,
    /// Restart the named server.
    Restart {
        /// Name of the server to restart.
        name: String,
    },
    /// Return recent log lines.
    Logs {
        /// Limit output to this server; `None` means all servers.
        server: Option<String>,
        /// Maximum number of lines to return.
        lines: usize,
    },
    /// Reload the config file and apply changes without restarting the daemon.
    Reload,
}

/// A response returned by the daemon for a single `DaemonRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    /// `true` when the request was handled successfully.
    pub ok: bool,
    /// Response payload (present on success when there is data to return).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Human-readable error message (present on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DaemonResponse {
    /// Successful response with a JSON payload.
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// Successful response with no payload.
    pub fn ok_empty() -> Self {
        Self {
            ok: true,
            data: None,
            error: None,
        }
    }

    /// Error response with a human-readable message.
    pub fn err(message: String) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(message),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared daemon state
// ─────────────────────────────────────────────────────────────────────────────

/// State shared between all concurrent connection handlers inside the daemon.
pub struct DaemonState {
    /// Handles to all managed server supervisor tasks.
    pub handles: Arc<tokio::sync::Mutex<Vec<ServerHandle>>>,
    /// Aggregated log buffers for all managed servers.
    pub log_agg: Arc<LogAggregator>,
    /// Cancelled when the daemon receives a `Stop` command or SIGTERM.
    pub shutdown: CancellationToken,
    /// Whether to use ANSI colors in log output.
    /// Reserved for future use when log output is colorized in the daemon.
    #[allow(dead_code)]
    pub color: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Server-side socket listener (Unix only)
// ─────────────────────────────────────────────────────────────────────────────

/// Default time a client may take to deliver its request line.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Default maximum length (in bytes) of a request line, excluding the newline.
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 64 * 1024;

/// After rejecting an oversized request, discard at most this many further bytes...
#[cfg(unix)]
const OVERSIZE_DRAIN_BYTES: u64 = 1024 * 1024;

/// ...and for at most this long, so the error response reaches the client.
#[cfg(unix)]
const OVERSIZE_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// Per-connection limits enforced by the control socket.
#[derive(Debug, Clone, Copy)]
pub struct ControlLimits {
    /// Maximum time to wait for a complete request line.
    pub read_timeout: Duration,
    /// Maximum length of a request line in bytes.
    pub max_request_bytes: usize,
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            read_timeout: DEFAULT_READ_TIMEOUT,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        }
    }
}

#[cfg(unix)]
/// Bind a Unix domain socket at `sock_path` and accept control connections.
///
/// Each connection receives exactly one request and sends exactly one response
/// (newline-delimited JSON). The listener shuts down when `state.shutdown` is
/// cancelled and removes the socket file on exit.
///
/// The socket is restricted to the owner (`0600`) right after binding, idle
/// clients are dropped after [`DEFAULT_READ_TIMEOUT`], and request lines are
/// capped at [`DEFAULT_MAX_REQUEST_BYTES`].
pub async fn run_control_socket(sock_path: &Path, state: Arc<DaemonState>) -> anyhow::Result<()> {
    run_control_socket_with_limits(sock_path, state, ControlLimits::default()).await
}

#[cfg(unix)]
/// Like [`run_control_socket`], but with explicit per-connection limits.
pub async fn run_control_socket_with_limits(
    sock_path: &Path,
    state: Arc<DaemonState>,
    limits: ControlLimits,
) -> anyhow::Result<()> {
    use tokio::net::UnixListener;

    // Remove any leftover socket from a previous run.
    let _ = tokio::fs::remove_file(sock_path).await;

    let listener = UnixListener::bind(sock_path)
        .map_err(|e| anyhow::anyhow!("Failed to bind socket {}: {e}", sock_path.display()))?;

    // Restrict the socket to the owner. The state directory is already 0700,
    // so there is no window in which another user can reach the socket.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |e| {
                anyhow::anyhow!(
                    "Failed to set permissions 0600 on socket {}: {e}",
                    sock_path.display()
                )
            },
        )?;
    }

    tracing::info!("Control socket listening on {}", sock_path.display());

    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let state = Arc::clone(&state);
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, state, limits).await {
                                tracing::warn!("Control socket connection error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to accept control socket connection: {e}");
                    }
                }
            }
            _ = state.shutdown.cancelled() => {
                tracing::debug!("Control socket shutting down");
                break;
            }
        }
    }

    // Remove the socket file on clean shutdown.
    let _ = tokio::fs::remove_file(sock_path).await;
    Ok(())
}

#[cfg(not(unix))]
/// Stub: Unix domain socket IPC is not supported on non-Unix platforms.
pub async fn run_control_socket(_sock_path: &Path, _state: Arc<DaemonState>) -> anyhow::Result<()> {
    anyhow::bail!("Unix domain socket IPC is not supported on this platform. Use foreground mode.")
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-connection handler (Unix only)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
/// Read one request line, dispatch it, and write one response line.
///
/// The request must arrive within `limits.read_timeout` and be at most
/// `limits.max_request_bytes` long; idle clients are dropped silently and
/// oversized requests get an error response before the connection is closed.
async fn handle_connection(
    stream: tokio::net::UnixStream,
    state: Arc<DaemonState>,
    limits: ControlLimits,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    // Allow one byte more than the cap so an over-long line is detectable
    // without ever buffering more than `max_request_bytes + 1` bytes.
    let cap = u64::try_from(limits.max_request_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut reader = BufReader::new(reader.take(cap));

    let mut buf = Vec::new();
    let read = tokio::time::timeout(limits.read_timeout, reader.read_until(b'\n', &mut buf)).await;
    let n = match read {
        Ok(result) => result?,
        Err(_elapsed) => {
            tracing::debug!(
                "Control socket client sent no complete request within {:?}; dropping",
                limits.read_timeout
            );
            return Ok(());
        }
    };

    if n == 0 {
        anyhow::bail!("Client disconnected without sending request");
    }

    let has_newline = buf.last() == Some(&b'\n');
    let line_len = if has_newline {
        buf.len() - 1
    } else {
        buf.len()
    };
    if line_len > limits.max_request_bytes {
        let response = DaemonResponse::err(format!(
            "Request too large (limit {} bytes)",
            limits.max_request_bytes
        ));
        let mut response_json = serde_json::to_string(&response)?;
        response_json.push('\n');
        // The client may already be gone; the error is still reported below.
        writer.write_all(response_json.as_bytes()).await?;
        writer.flush().await?;
        // Closing a socket with unread input makes the kernel send RST, which
        // can destroy the error response before the client reads it. Briefly
        // drain (bounded in both bytes and time) so the client sees the reply.
        let rest = reader.into_inner().into_inner();
        let mut bounded = rest.take(OVERSIZE_DRAIN_BYTES);
        let _ = tokio::time::timeout(
            OVERSIZE_DRAIN_TIMEOUT,
            tokio::io::copy(&mut bounded, &mut tokio::io::sink()),
        )
        .await;
        anyhow::bail!(
            "Request exceeded {} bytes; connection closed",
            limits.max_request_bytes
        );
    }

    let request: DaemonRequest =
        serde_json::from_slice(&buf).map_err(|e| anyhow::anyhow!("Invalid request JSON: {e}"))?;

    let response = dispatch_request(request, &state).await;

    let mut response_json = serde_json::to_string(&response)?;
    response_json.push('\n');
    writer.write_all(response_json.as_bytes()).await?;
    writer.flush().await?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Request dispatch (Unix only)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
/// Route a `DaemonRequest` to the appropriate handler and return a response.
async fn dispatch_request(request: DaemonRequest, state: &DaemonState) -> DaemonResponse {
    match request {
        DaemonRequest::Status => {
            let handles = state.handles.lock().await;
            let statuses: Vec<_> = handles
                .iter()
                .map(|h| {
                    let snapshot = h.state_rx.borrow().clone();
                    serde_json::json!({
                        "name": h.name,
                        "state": snapshot.process_state.to_string(),
                        "health": snapshot.health.to_string(),
                        "pid": snapshot.pid,
                        "restart_count": snapshot.restart_count,
                        "transport": snapshot.transport,
                        "tools": snapshot.capabilities.tools.len(),
                        "tool_names": snapshot.capabilities.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
                        "resources": snapshot.capabilities.resources.len(),
                        "resource_names": snapshot.capabilities.resources.iter().map(|r| &r.name).collect::<Vec<_>>(),
                        "prompts": snapshot.capabilities.prompts.len(),
                        "prompt_names": snapshot.capabilities.prompts.iter().map(|p| &p.name).collect::<Vec<_>>(),
                    })
                })
                .collect();
            DaemonResponse::success(serde_json::json!(statuses))
        }

        DaemonRequest::Stop => {
            tracing::info!("Stop command received via control socket");
            state.shutdown.cancel();
            DaemonResponse::ok_empty()
        }

        DaemonRequest::Restart { name } => {
            let handles = state.handles.lock().await;
            match crate::supervisor::restart_server(&handles, &name).await {
                Ok(()) => DaemonResponse::ok_empty(),
                Err(e) => DaemonResponse::err(e.to_string()),
            }
        }

        DaemonRequest::Logs { server, lines } => match &server {
            Some(name) => match state.log_agg.get_buffer(name) {
                Some(buf) => {
                    let log_lines = buf.snapshot_last(lines).await;
                    let formatted: Vec<_> = log_lines
                        .iter()
                        .map(|l| crate::logs::format_log_line(l, false))
                        .collect();
                    DaemonResponse::success(serde_json::json!(formatted))
                }
                None => DaemonResponse::err(format!("Unknown server: '{name}'")),
            },
            None => {
                let all = state.log_agg.snapshot_all().await;
                let tail = if all.len() > lines {
                    &all[all.len() - lines..]
                } else {
                    &all[..]
                };
                let formatted: Vec<_> = tail
                    .iter()
                    .map(|l| crate::logs::format_log_line(l, false))
                    .collect();
                DaemonResponse::success(serde_json::json!(formatted))
            }
        },

        DaemonRequest::Reload => {
            // Send SIGHUP to self so the main event loop's sighup handler triggers
            // the config reload without requiring a separate reload channel.
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let pid = Pid::from_raw(std::process::id() as i32);
                match kill(pid, Signal::SIGHUP) {
                    Ok(()) => DaemonResponse::ok_empty(),
                    Err(e) => DaemonResponse::err(format!("Failed to send SIGHUP: {e}")),
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Client-side helper
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
/// Connect to a running daemon and send a single request, returning the response.
///
/// Both the connection and the response read are bounded by `timeout_secs`.
pub async fn send_daemon_command(
    sock_path: &Path,
    request: &DaemonRequest,
    timeout_secs: u64,
) -> anyhow::Result<DaemonResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        UnixStream::connect(sock_path),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Connection to daemon timed out after {timeout_secs}s"))?
    .map_err(|e| {
        anyhow::anyhow!(
            "Cannot connect to daemon socket ({}): {e}\nIs the daemon running? Start with: mcp-hub start --daemon",
            sock_path.display()
        )
    })?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    let mut json = serde_json::to_string(request)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    let response_line = tokio::time::timeout(Duration::from_secs(timeout_secs), lines.next_line())
        .await
        .map_err(|_| anyhow::anyhow!("Daemon response timed out after {timeout_secs}s"))?
        .map_err(|e| anyhow::anyhow!("Error reading daemon response: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("Daemon closed connection without responding"))?;

    let response: DaemonResponse = serde_json::from_str(&response_line)
        .map_err(|e| anyhow::anyhow!("Invalid daemon response JSON: {e}"))?;

    Ok(response)
}

#[cfg(not(unix))]
/// Stub: Unix domain socket IPC is not supported on non-Unix platforms.
pub async fn send_daemon_command(
    _sock_path: &Path,
    _request: &DaemonRequest,
    _timeout_secs: u64,
) -> anyhow::Result<DaemonResponse> {
    anyhow::bail!("Unix domain socket IPC is not supported on this platform. Use foreground mode.")
}
