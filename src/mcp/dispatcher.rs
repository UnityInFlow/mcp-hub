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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex};

use crate::mcp::protocol::JsonRpcResponse;

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
// Reader task
// ─────────────────────────────────────────────────────────────────────────────

/// Long-running task that owns `stdout` and routes JSON-RPC responses by ID.
///
/// - Each line parsed as a valid `JsonRpcResponse` is matched against `pending`
///   by `id`. The corresponding oneshot sender is removed from the map and fired.
/// - Lines that fail to parse (notifications, log output, etc.) are discarded
///   via `tracing::debug!` to prevent pipe-buffer backpressure.
/// - When stdout closes (process exited), all remaining pending senders are
///   drained (dropped) so their receivers get `RecvError` instead of hanging.
///
/// This task must be spawned exactly once per process instance. It must be
/// cancelled (by killing/closing the process) before spawning a new one on restart.
pub async fn reader_task(stdout: ChildStdout, pending: PendingMap) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str::<JsonRpcResponse>(&line) {
            Ok(response) => {
                let mut map = pending.lock().await;
                if let Some(sender) = map.remove(&response.id) {
                    // Ignore send errors — the waiter may have timed out and moved on.
                    let _ = sender.send(response);
                } else {
                    tracing::debug!(id = response.id, "Received response with no pending waiter");
                }
            }
            Err(_) => {
                // Non-JSON-RPC line (notification, server log, etc.) — discard.
                tracing::debug!("Non-JSON-RPC stdout line (discarded)");
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

/// Write a JSON-RPC request to `stdin` and await the matching response.
///
/// Registers a oneshot channel in `pending` **before** writing the request so
/// no response can be lost due to a race. Removes the entry from `pending` on
/// timeout so the map does not leak entries.
///
/// # Errors
/// - Serialization fails (impossible for well-formed types but propagated).
/// - Writing to stdin fails (process died).
/// - The reader task exited before a response arrived (`RecvError`).
/// - No response arrived within `timeout_secs`.
pub async fn send_request<T: serde::Serialize>(
    stdin: &SharedStdin,
    pending: &PendingMap,
    id: u64,
    request: &T,
    timeout_secs: u64,
) -> anyhow::Result<JsonRpcResponse> {
    let (tx, rx) = oneshot::channel();

    // Insert the waiter before writing so the reader task cannot beat us to the response.
    {
        let mut map = pending.lock().await;
        map.insert(id, tx);
    }

    // Serialize request to newline-delimited JSON.
    let mut json = serde_json::to_string(request)
        .map_err(|e| anyhow::anyhow!("Failed to serialize request: {e}"))?;
    json.push('\n');

    // Write and flush under the stdin lock.
    {
        let mut stdin_lock = stdin.lock().await;
        stdin_lock
            .write_all(json.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write to stdin: {e}"))?;
        stdin_lock
            .flush()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to flush stdin: {e}"))?;
    }

    // Await the response with a timeout.
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await;

    match result {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_recv_err)) => {
            // The reader task dropped the sender (stdout closed).
            anyhow::bail!("Reader task closed before response for id={id}")
        }
        Err(_timeout) => {
            // Timed out — clean up the pending entry to prevent a map leak.
            let mut map = pending.lock().await;
            map.remove(&id);
            anyhow::bail!("Request id={id} timed out after {timeout_secs}s")
        }
    }
}

/// Write a JSON-RPC notification to `stdin` (fire-and-forget — no response expected).
///
/// Unlike `send_request`, this does not register a pending waiter and does not
/// block waiting for a reply.
///
/// # Errors
/// - Serialization fails.
/// - Writing to stdin fails (process died).
#[allow(dead_code)] // used in Phase 3 Plan 02 for notifications/initialized
pub async fn send_notification<T: serde::Serialize>(
    stdin: &SharedStdin,
    notification: &T,
) -> anyhow::Result<()> {
    let mut json = serde_json::to_string(notification)
        .map_err(|e| anyhow::anyhow!("Failed to serialize notification: {e}"))?;
    json.push('\n');

    let mut stdin_lock = stdin.lock().await;
    stdin_lock
        .write_all(json.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write notification to stdin: {e}"))?;
    stdin_lock
        .flush()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to flush stdin: {e}"))?;

    Ok(())
}
