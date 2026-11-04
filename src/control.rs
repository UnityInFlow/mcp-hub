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

use crate::config::HubConfig;
use crate::logs::LogAggregator;
use crate::supervisor::ServerHandle;
use crate::web::SharedServerViews;

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
    /// Stream log lines until the client disconnects or the daemon shuts down.
    ///
    /// The daemon answers with one ordinary [`DaemonResponse`] line (ok, or an
    /// error such as an unknown server) and, on success, keeps the connection
    /// open and writes one [`FollowEvent`] JSON object per line: the last
    /// `lines` buffered lines first, then live lines.
    LogsFollow {
        /// Limit output to this server; `None` means all servers.
        server: Option<String>,
        /// Number of buffered lines to send before switching to live lines.
        lines: usize,
    },
}

/// One event on a `LogsFollow` stream (after the initial [`DaemonResponse`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum FollowEvent {
    /// A formatted log line, identical to what `logs` prints.
    Line {
        /// The formatted line (`server | timestamp message`).
        text: String,
    },
    /// The client fell behind and the daemon dropped log lines.
    Lagged {
        /// How many lines were dropped.
        dropped: u64,
    },
}

impl FollowEvent {
    /// Text the client prints for this event.
    pub fn render(&self) -> String {
        match self {
            FollowEvent::Line { text } => text.clone(),
            FollowEvent::Lagged { dropped } => {
                format!("[mcp-hub] \u{2026} {dropped} log lines dropped (client too slow)")
            }
        }
    }
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
    /// Explicit `--config` path used by reloads (`None` = default search).
    pub config_path: Option<std::path::PathBuf>,
    /// The config the running servers were started from; reloads diff against it.
    ///
    /// Holding this lock serialises reloads (SIGHUP and `Reload` requests).
    pub current_config: tokio::sync::Mutex<HubConfig>,
    /// Snapshot views published to the web UI; refreshed after every reload.
    pub views: SharedServerViews,
}

/// What a successful config reload changed (server counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReloadSummary {
    /// Servers newly started.
    pub added: usize,
    /// Servers stopped because they left the config.
    pub removed: usize,
    /// Servers restarted because their config changed.
    pub changed: usize,
    /// Servers left running untouched.
    pub unchanged: usize,
}

impl std::fmt::Display for ReloadSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} added, {} removed, {} changed, {} unchanged",
            self.added, self.removed, self.changed, self.unchanged
        )
    }
}

/// Reload the config from disk and apply the difference to the running servers.
///
/// Used by both the `Reload` control request and the SIGHUP handler. On error
/// (unreadable or invalid config) nothing is touched: the servers keep running
/// from the previous config.
pub async fn reload_config(state: &DaemonState) -> anyhow::Result<ReloadSummary> {
    // Held for the whole reload so concurrent reloads cannot interleave.
    let mut current = state.current_config.lock().await;

    let new_config = crate::config::find_and_load_config(state.config_path.as_deref())?;

    let (added, removed, changed) = crate::supervisor::apply_config_diff(
        &state.handles,
        &current,
        &new_config,
        &state.shutdown,
        &state.log_agg,
    )
    .await;

    // Publish the new handle set to the web UI only now, with a brief lock;
    // until here it kept serving the previous snapshots.
    {
        let handles = state.handles.lock().await;
        crate::web::publish_server_views(&state.views, &handles).await;
    }

    let summary = ReloadSummary {
        added,
        removed,
        changed,
        unchanged: new_config.servers.len().saturating_sub(added + changed),
    };
    *current = new_config;
    tracing::info!(
        added = summary.added,
        removed = summary.removed,
        changed = summary.changed,
        unchanged = summary.unchanged,
        "Config reload complete"
    );
    Ok(summary)
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

/// A bound control socket listener, ready to be served.
#[cfg(unix)]
pub type ControlListener = tokio::net::UnixListener;

/// Placeholder: Unix domain sockets are not available on this platform.
#[cfg(not(unix))]
pub struct ControlListener;

#[cfg(unix)]
/// Bind a Unix domain socket at `sock_path`, replacing any leftover socket file.
///
/// Split from [`serve_control_socket`] so the daemon can tell its parent
/// "ready" only after the bind has actually succeeded.
pub async fn bind_control_socket(sock_path: &Path) -> anyhow::Result<ControlListener> {
    // Remove any leftover socket from a previous run.
    let _ = tokio::fs::remove_file(sock_path).await;

    let listener = ControlListener::bind(sock_path)
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
    Ok(listener)
}

#[cfg(not(unix))]
/// Stub: Unix domain socket IPC is not supported on non-Unix platforms.
pub async fn bind_control_socket(_sock_path: &Path) -> anyhow::Result<ControlListener> {
    anyhow::bail!("Unix domain socket IPC is not supported on this platform. Use foreground mode.")
}

#[cfg(unix)]
/// Bind a Unix domain socket at `sock_path` and accept control connections.
///
/// Convenience wrapper over [`bind_control_socket`] + [`serve_control_socket`].
/// The daemon binary binds and serves separately; this is the library entry point.
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
    let listener = bind_control_socket(sock_path).await?;
    serve_control_socket_with_limits(listener, sock_path, state, limits).await
}

#[cfg(unix)]
/// Accept control connections on an already-bound `listener`.
///
/// Each connection receives exactly one request and sends exactly one response
/// (newline-delimited JSON). The listener shuts down when `state.shutdown` is
/// cancelled and removes the socket file on exit.
pub async fn serve_control_socket(
    listener: ControlListener,
    sock_path: &Path,
    state: Arc<DaemonState>,
) -> anyhow::Result<()> {
    serve_control_socket_with_limits(listener, sock_path, state, ControlLimits::default()).await
}

#[cfg(unix)]
/// Like [`serve_control_socket`], but with explicit per-connection limits:
/// idle clients are dropped after `limits.read_timeout` and request lines are
/// capped at `limits.max_request_bytes`.
pub async fn serve_control_socket_with_limits(
    listener: ControlListener,
    sock_path: &Path,
    state: Arc<DaemonState>,
    limits: ControlLimits,
) -> anyhow::Result<()> {
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

#[cfg(not(unix))]
/// Stub: Unix domain socket IPC is not supported on non-Unix platforms.
pub async fn serve_control_socket(
    _listener: ControlListener,
    _sock_path: &Path,
    _state: Arc<DaemonState>,
) -> anyhow::Result<()> {
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

    if let DaemonRequest::LogsFollow { server, lines } = request {
        // The read timeout and size cap above only guard the request line; the
        // stream below is deliberately unbounded in time.
        return follow_logs(reader, writer, &state, server, lines).await;
    }

    let response = dispatch_request(request, &state).await;

    let mut response_json = serde_json::to_string(&response)?;
    response_json.push('\n');
    writer.write_all(response_json.as_bytes()).await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(unix)]
/// Serve a `LogsFollow` request: ack, replay the tail, then stream live lines.
///
/// Returns when the client disconnects, a write fails, or the daemon shuts down.
async fn follow_logs<R>(
    mut reader: R,
    mut writer: tokio::net::unix::OwnedWriteHalf,
    state: &DaemonState,
    server: Option<String>,
    lines: usize,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::broadcast::error::RecvError;

    async fn send_json<T: Serialize>(
        w: &mut tokio::net::unix::OwnedWriteHalf,
        value: &T,
    ) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(value)?;
        json.push('\n');
        w.write_all(json.as_bytes()).await?;
        w.flush().await?;
        Ok(())
    }

    // Subscribe before taking the snapshot so no line falls in the gap.
    let mut rx = state.log_agg.subscribe();

    let mut backlog = match &server {
        Some(name) => match state.log_agg.get_buffer(name) {
            Some(buf) => buf.snapshot_last(lines).await,
            None => {
                let resp = DaemonResponse::err(format!("Unknown server: '{name}'"));
                send_json(&mut writer, &resp).await?;
                return Ok(());
            }
        },
        None => {
            let all = state.log_agg.snapshot_all().await;
            let start = all.len().saturating_sub(lines);
            all[start..].to_vec()
        }
    };

    // A client that vanished mid-stream is a normal end, not an error.
    if send_json(&mut writer, &DaemonResponse::ok_empty())
        .await
        .is_err()
    {
        return Ok(());
    }
    for line in &backlog {
        let event = FollowEvent::Line {
            text: crate::logs::format_log_line(line, false),
        };
        if send_json(&mut writer, &event).await.is_err() {
            return Ok(());
        }
    }

    let mut scratch = [0u8; 256];
    loop {
        let event = tokio::select! {
            () = state.shutdown.cancelled() => return Ok(()),
            // Any EOF or error on the read half means the client went away.
            read = reader.read(&mut scratch) => match read {
                Ok(0) | Err(_) => return Ok(()),
                Ok(_) => continue, // stray client bytes are ignored
            },
            received = rx.recv() => match received {
                Ok(line) => {
                    if server.as_deref().is_some_and(|name| line.server != name) {
                        continue;
                    }
                    // Drop lines already replayed from the snapshot (they were
                    // pushed between subscribing and snapshotting).
                    if let Some(pos) = backlog.iter().position(|b| {
                        b.server == line.server
                            && b.timestamp == line.timestamp
                            && b.message == line.message
                    }) {
                        backlog.swap_remove(pos);
                        continue;
                    }
                    FollowEvent::Line { text: crate::logs::format_log_line(&line, false) }
                }
                Err(RecvError::Lagged(n)) => FollowEvent::Lagged { dropped: n },
                Err(RecvError::Closed) => return Ok(()),
            },
        };
        if send_json(&mut writer, &event).await.is_err() {
            return Ok(());
        }
    }
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

        // Handled by `handle_connection` (needs the connection); never routed here.
        DaemonRequest::LogsFollow { .. } => {
            DaemonResponse::err("logs_follow is a streaming request".to_string())
        }

        DaemonRequest::Reload => match reload_config(state).await {
            Ok(summary) => match serde_json::to_value(summary) {
                Ok(data) => DaemonResponse::success(data),
                Err(e) => DaemonResponse::err(format!("Failed to encode reload summary: {e}")),
            },
            Err(e) => {
                tracing::error!("Config reload failed: {e:#}");
                DaemonResponse::err(format!("{e:#}"))
            }
        },
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

#[cfg(unix)]
/// Open a streaming request: send `request`, read the initial response, and
/// return the still-open line reader for the follow-up events.
///
/// Only the connect and the first response are bounded by `timeout_secs`; the
/// stream itself is not. A failure response becomes an error.
pub async fn open_daemon_stream(
    sock_path: &Path,
    request: &DaemonRequest,
    timeout_secs: u64,
) -> anyhow::Result<DaemonStream> {
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

    let first = tokio::time::timeout(Duration::from_secs(timeout_secs), lines.next_line())
        .await
        .map_err(|_| anyhow::anyhow!("Daemon response timed out after {timeout_secs}s"))?
        .map_err(|e| anyhow::anyhow!("Error reading daemon response: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("Daemon closed connection without responding"))?;
    let response: DaemonResponse = serde_json::from_str(&first)
        .map_err(|e| anyhow::anyhow!("Invalid daemon response JSON: {e}"))?;
    if !response.ok {
        anyhow::bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
    Ok(DaemonStream {
        lines,
        _writer: writer,
    })
}

#[cfg(unix)]
/// An open streaming connection to the daemon (see [`open_daemon_stream`]).
pub struct DaemonStream {
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
    /// Held open: closing the write half would look like a disconnect to the daemon.
    _writer: tokio::net::unix::OwnedWriteHalf,
}

#[cfg(unix)]
impl DaemonStream {
    /// Next event from the daemon; `None` at end of stream.
    pub async fn next_event(&mut self) -> anyhow::Result<Option<FollowEvent>> {
        match self.lines.next_line().await? {
            Some(line) => {
                Ok(Some(serde_json::from_str(&line).map_err(|e| {
                    anyhow::anyhow!("Invalid daemon stream event JSON: {e}")
                })?))
            }
            None => Ok(None),
        }
    }
}

#[cfg(not(unix))]
/// Stub: streaming is not supported on non-Unix platforms.
pub async fn open_daemon_stream(
    _sock_path: &Path,
    _request: &DaemonRequest,
    _timeout_secs: u64,
) -> anyhow::Result<DaemonStream> {
    anyhow::bail!("Unix domain socket IPC is not supported on this platform. Use foreground mode.")
}

#[cfg(not(unix))]
/// Placeholder: streaming is not available on this platform.
pub struct DaemonStream;

#[cfg(not(unix))]
impl DaemonStream {
    /// Never reached: [`open_daemon_stream`] always fails on this platform.
    pub async fn next_event(&mut self) -> anyhow::Result<Option<FollowEvent>> {
        Ok(None)
    }
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

#[cfg(test)]
mod follow_event_tests {
    use super::*;

    #[test]
    fn lagged_marker_is_visible_and_counts_lines() {
        let text = FollowEvent::Lagged { dropped: 42 }.render();
        assert!(text.starts_with("[mcp-hub]"), "{text}");
        assert!(
            text.contains("42 log lines dropped (client too slow)"),
            "{text}"
        );
    }

    #[test]
    fn follow_events_round_trip_as_one_json_line() {
        for ev in [
            FollowEvent::Line {
                text: "a | t m".into(),
            },
            FollowEvent::Lagged { dropped: 3 },
        ] {
            let json = serde_json::to_string(&ev).expect("serialise");
            assert!(!json.contains('\n'));
            let back: FollowEvent = serde_json::from_str(&json).expect("parse");
            assert_eq!(back, ev);
        }
    }

    #[test]
    fn line_event_renders_text_unchanged() {
        assert_eq!(FollowEvent::Line { text: "x".into() }.render(), "x");
    }
}
