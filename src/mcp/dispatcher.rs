//! Shared JSON-RPC dispatcher for concurrent MCP communication over stdio.
//!
//! The dispatcher pattern replaces the per-request read-in-place approach from Phase 2.
//! A single `reader_task` owns stdout and routes responses by `id` via a `PendingMap`.
//! Callers write requests via `send_request`, which inserts a oneshot channel into the
//! map before writing, then awaits the response (with timeout). Notifications are sent
//! fire-and-forget via `send_notification`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::mcp::protocol::{JsonRpcId, JsonRpcResponse};

/// Map of pending request IDs to their oneshot response senders.
///
/// Shared between `reader_task` (which removes entries when a response arrives)
/// and `send_request` (which inserts entries before writing the request).
pub type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>;

/// Arc-wrapped mutex around the child's stdin handle.
///
/// Shared between the health check loop and the future introspection task so
/// both can send requests without racing on the pipe.
pub type SharedStdin = Arc<Mutex<ChildStdin>>;

/// Longest line (in bytes) accepted from a server's stdout or stderr.
///
/// Longer lines are skipped rather than buffered, so a server that never emits
/// a newline cannot grow the hub's memory without bound.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// How long a notification or a reply to a server-initiated request may take
/// to be written to the server's stdin.
const AUX_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Replies to server-initiated requests that may be queued for the writer.
const REPLY_QUEUE_CAPACITY: usize = 64;

/// JSON-RPC error code for "Method not found".
const METHOD_NOT_FOUND: i64 = -32601;

// ─────────────────────────────────────────────────────────────────────────────
// ID allocator
// ─────────────────────────────────────────────────────────────────────────────

/// Per-server monotonic request ID allocator.
///
/// Using `AtomicU64` avoids lock contention when multiple concurrent tasks
/// need fresh IDs (health pings + introspection requests).
pub struct IdAllocator {
    counter: AtomicU64,
}

impl IdAllocator {
    /// Create a new allocator starting at ID 1.
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Return the next unique request ID and advance the counter.
    pub fn next_id(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded line reading
// ─────────────────────────────────────────────────────────────────────────────

/// One line read by [`read_bounded_line`].
#[derive(Debug, PartialEq, Eq)]
pub enum BoundedLine {
    /// A complete line (without its terminator), decoded lossily as UTF-8.
    Line(String),
    /// The line exceeded the limit; its bytes were discarded up to and
    /// including the terminating newline (or EOF).
    Oversized,
}

/// Read the next line from `reader`, holding at most `max_bytes` of it in memory.
///
/// Unlike `AsyncBufReadExt::lines`, this neither grows without bound on a line
/// that never ends nor fails on invalid UTF-8 (invalid sequences become
/// U+FFFD). Only a genuine I/O error is returned as `Err`; `Ok(None)` is EOF.
pub async fn read_bounded_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<BoundedLine>>
where
    R: AsyncBufRead + Unpin,
{
    let mut buf: Vec<u8> = Vec::new();
    let mut overflowed = false;
    let mut saw_any = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // EOF: flush a trailing unterminated line, if any.
            return Ok(match (saw_any, overflowed) {
                (false, _) => None,
                (true, true) => Some(BoundedLine::Oversized),
                (true, false) => Some(finish_line(buf)),
            });
        }
        saw_any = true;

        let (chunk_len, found_newline) = match available.iter().position(|b| *b == b'\n') {
            Some(pos) => (pos, true),
            None => (available.len(), false),
        };
        if !overflowed {
            if buf.len() + chunk_len > max_bytes {
                overflowed = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&available[..chunk_len]);
            }
        }
        // Consume the chunk plus the newline itself when present.
        reader.consume(chunk_len + usize::from(found_newline));

        if found_newline {
            return Ok(Some(if overflowed {
                BoundedLine::Oversized
            } else {
                finish_line(buf)
            }));
        }
    }
}

fn finish_line(mut buf: Vec<u8>) -> BoundedLine {
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    BoundedLine::Line(String::from_utf8_lossy(&buf).into_owned())
}

// ─────────────────────────────────────────────────────────────────────────────
// Reader task
// ─────────────────────────────────────────────────────────────────────────────

/// Deliver a validated response to the waiter registered under its id.
///
/// The hub only sends numeric ids, so a string id can never match a waiter.
async fn route_response(pending: &PendingMap, response: JsonRpcResponse) {
    match response.id {
        JsonRpcId::Number(id) => {
            let mut map = pending.lock().await;
            if let Some(sender) = map.remove(&id) {
                // Ignore send errors — the waiter may have timed out and moved on.
                let _ = sender.send(response);
            } else {
                tracing::debug!(id, "Received response with no pending waiter");
            }
        }
        JsonRpcId::String(ref id) => {
            tracing::debug!(id = %id, "Received response with string id (hub only sends numeric ids)");
        }
    }
}

/// If `line` is a server-initiated JSON-RPC request (has both `method` and a
/// numeric or string `id`), build the `-32601 Method not found` reply for it.
///
/// Notifications (no `id`) and anything that is not a JSON object get no reply.
fn method_not_found_reply(line: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let object = value.as_object()?;
    object.get("method")?;
    let id = object.get("id")?;
    match id {
        serde_json::Value::Number(_) | serde_json::Value::String(_) => {}
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Array(_)
        | serde_json::Value::Object(_) => return None,
    }
    let reply = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": METHOD_NOT_FOUND, "message": "Method not found" },
    });
    serde_json::to_string(&reply).ok()
}

/// Long-running task that owns `stdout` and routes JSON-RPC responses by ID.
///
/// - Each line parsed as a valid `JsonRpcResponse` is matched against `pending`
///   by `id`. The corresponding oneshot sender is removed from the map and fired.
/// - Server-initiated requests (a `method` plus an `id`, e.g. `roots/list`) are
///   answered with a `-32601 Method not found` error written to `stdin`, so a
///   server blocking on the reply does not hang. The hub advertises no client
///   capabilities, so there is nothing to serve. Replies go through a bounded
///   queue and a separate writer task, so a stuck stdin never stalls routing.
/// - Other lines (notifications, log output, etc.) are discarded via
///   `tracing::debug!`.
/// - Lines longer than [`MAX_LINE_BYTES`] are skipped and invalid UTF-8 is
///   decoded lossily: neither ends the loop. Only EOF or a real I/O error does.
/// - When stdout closes (process exited), all remaining pending senders are
///   drained (dropped) so their receivers get `RecvError` instead of hanging.
///
/// This task must be spawned exactly once per process instance. It must be
/// cancelled (by killing/closing the process) before spawning a new one on restart.
pub async fn reader_task(stdout: ChildStdout, pending: PendingMap, stdin: SharedStdin) {
    let mut reader = BufReader::new(stdout);

    let (reply_tx, mut reply_rx) = mpsc::channel::<String>(REPLY_QUEUE_CAPACITY);
    tokio::spawn(async move {
        while let Some(mut reply) = reply_rx.recv().await {
            reply.push('\n');
            if let Err(err) = write_line(&stdin, reply.as_bytes(), AUX_WRITE_TIMEOUT).await {
                tracing::debug!("Could not reply to server-initiated request: {err}");
            }
        }
    });

    loop {
        match read_bounded_line(&mut reader, MAX_LINE_BYTES).await {
            Ok(Some(BoundedLine::Line(line))) => {
                match serde_json::from_str::<JsonRpcResponse>(&line) {
                    Ok(response) => route_response(&pending, response).await,
                    Err(err) => {
                        // Not a valid response: log output, a notification, or a
                        // server-initiated request (e.g. `roots/list`). None of these
                        // may be routed to a pending waiter, even if the id collides.
                        tracing::debug!("Non-response stdout line (discarded): {err}");
                        if let Some(reply) = method_not_found_reply(&line) {
                            if reply_tx.try_send(reply).is_err() {
                                tracing::debug!("Reply queue full or closed; dropping reply");
                            }
                        }
                    }
                }
            }
            Ok(Some(BoundedLine::Oversized)) => {
                tracing::warn!("Skipped stdout line longer than {MAX_LINE_BYTES} bytes");
            }
            Ok(None) => break,
            Err(err) => {
                tracing::warn!("Reader task stopping on stdout read error: {err}");
                break;
            }
        }
    }

    // stdout closed — drain all pending waiters so receivers get RecvError
    // instead of blocking indefinitely.
    let mut map = pending.lock().await;
    map.drain();
    // Dropping the senders causes Receivers to return Err(RecvError).
    tracing::debug!("Reader task exiting — stdout closed, drained pending map");
}

// ─────────────────────────────────────────────────────────────────────────────
// Send helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Write `bytes` to the shared stdin and flush, all within `timeout`.
///
/// The timeout covers acquiring the stdin lock as well as the write, so a
/// server that stops reading its stdin (full pipe) cannot block callers forever.
/// If the timeout fires mid-write the stream may be left with a partial line;
/// that only happens when the server is already not draining its stdin.
async fn write_line(stdin: &SharedStdin, bytes: &[u8], timeout: Duration) -> anyhow::Result<()> {
    let write = async {
        let mut stdin_lock = stdin.lock().await;
        stdin_lock.write_all(bytes).await?;
        stdin_lock.flush().await
    };
    match tokio::time::timeout(timeout, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => anyhow::bail!("Failed to write to stdin: {e}"),
        Err(_elapsed) => anyhow::bail!("Timed out after {}s writing to stdin", timeout.as_secs()),
    }
}

/// Removes a pending entry when dropped while still armed.
///
/// `send_request` disarms it after doing the removal explicitly; the drop path
/// only matters when the future is cancelled (e.g. dropped by a `select!`).
struct PendingGuard {
    pending: PendingMap,
    id: u64,
    armed: bool,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut map) = self.pending.try_lock() {
            map.remove(&self.id);
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let pending = Arc::clone(&self.pending);
            let id = self.id;
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}

/// Write a JSON-RPC request to `stdin` and await the matching response.
///
/// Registers a oneshot channel in `pending` **before** writing the request so
/// no response can be lost due to a race. The entry is removed on every exit
/// path (write error, write timeout, response timeout, cancellation), so the
/// map never leaks entries.
///
/// `timeout_secs` bounds the stdin write and the wait for the response
/// together.
///
/// # Errors
/// - Serialization fails (impossible for well-formed types but propagated).
/// - Writing to stdin fails (process died) or does not finish in time (the
///   server stopped reading).
/// - The reader task exited before a response arrived (`RecvError`).
/// - No response arrived within `timeout_secs`.
pub async fn send_request<T: serde::Serialize>(
    stdin: &SharedStdin,
    pending: &PendingMap,
    id: u64,
    request: &T,
    timeout_secs: u64,
) -> anyhow::Result<JsonRpcResponse> {
    // Serialize request to newline-delimited JSON before touching the map.
    let mut json = serde_json::to_string(request)
        .map_err(|e| anyhow::anyhow!("Failed to serialize request: {e}"))?;
    json.push('\n');

    let (tx, rx) = oneshot::channel();

    // Insert the waiter before writing so the reader task cannot beat us to the response.
    pending.lock().await.insert(id, tx);
    let mut guard = PendingGuard {
        pending: Arc::clone(pending),
        id,
        armed: true,
    };

    let timeout = Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    let result = async {
        write_line(stdin, json.as_bytes(), timeout).await?;
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(response)) => Ok(response),
            // The reader task dropped the sender (stdout closed).
            Ok(Err(_recv_err)) => anyhow::bail!("Reader task closed before response for id={id}"),
            Err(_elapsed) => anyhow::bail!("Request id={id} timed out after {timeout_secs}s"),
        }
    }
    .await;

    // Always clear our entry (a no-op when the reader already took it).
    pending.lock().await.remove(&id);
    guard.armed = false;

    result
}

/// Write a JSON-RPC notification to `stdin` (fire-and-forget — no response expected).
///
/// Unlike `send_request`, this does not register a pending waiter and does not
/// block waiting for a reply. The write itself is bounded by a short timeout.
///
/// # Errors
/// - Serialization fails.
/// - Writing to stdin fails (process died) or times out (server not reading).
pub async fn send_notification<T: serde::Serialize>(
    stdin: &SharedStdin,
    notification: &T,
) -> anyhow::Result<()> {
    let mut json = serde_json::to_string(notification)
        .map_err(|e| anyhow::anyhow!("Failed to serialize notification: {e}"))?;
    json.push('\n');

    write_line(stdin, json.as_bytes(), AUX_WRITE_TIMEOUT).await
}
