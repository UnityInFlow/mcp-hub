//! MCP health check loop — sends JSON-RPC pings via the shared dispatcher.
//!
//! Phase 3 refactor: `ping_server` and `run_health_check_loop` now accept
//! `SharedStdin` + `PendingMap` + `Arc<IdAllocator>` instead of owning
//! `ChildStdin`/`ChildStdout` directly. The `reader_task` in `dispatcher.rs`
//! owns stdout for the lifetime of the process.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::mcp::dispatcher::{IdAllocator, PendingMap, SharedStdin};
use crate::mcp::protocol::PingRequest;
use crate::types::{compute_health_status, HealthStatus, ServerSnapshot};

/// Default health check interval in seconds (configurable per server via config).
pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Send a single MCP JSON-RPC ping to the server and return latency in milliseconds.
///
/// Writes `PingRequest::new(id)` via the shared dispatcher and awaits the matching
/// response within a 5-second timeout. Returns the round-trip latency on success.
pub async fn ping_server(
    stdin: &SharedStdin,
    pending: &PendingMap,
    id: u64,
) -> anyhow::Result<u64> {
    let request = PingRequest::new(id);
    let start = std::time::Instant::now();

    let response =
        crate::mcp::dispatcher::send_request(stdin, pending, id, &request, 5).await?;

    if response.error.is_some() {
        anyhow::bail!(
            "Server returned error response to ping id={id}: {:?}",
            response.error
        );
    }

    Ok(start.elapsed().as_millis() as u64)
}

/// Run the per-server health check loop.
///
/// - Sends MCP JSON-RPC pings at `interval_secs` intervals via the shared dispatcher.
/// - Updates `snapshot_tx` with `HealthStatus::Healthy` on success, or transitions
///   through Degraded/Failed on consecutive misses via `compute_health_status`.
/// - Exits cleanly when `cancel` is triggered.
///
/// The `reader_task` in `dispatcher.rs` owns stdout for the process lifetime.
/// This task only writes pings via `SharedStdin` and receives responses via `PendingMap`.
/// The supervisor cancels the token before killing the process, and again on restart.
pub async fn run_health_check_loop(
    server_name: String,
    interval_secs: u64,
    stdin: SharedStdin,
    pending: PendingMap,
    id_alloc: Arc<IdAllocator>,
    snapshot_tx: tokio::sync::watch::Sender<ServerSnapshot>,
    cancel: CancellationToken,
) {
    use tokio::time::MissedTickBehavior;

    let mut consecutive_misses: u32 = 0;

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = cancel.cancelled() => {
                tracing::debug!(server = %server_name, "Health check loop cancelled");
                return;
            }
        }

        let request_id = id_alloc.next_id();

        match ping_server(&stdin, &pending, request_id).await {
            Ok(latency_ms) => {
                let prev_misses = consecutive_misses;
                consecutive_misses = 0;
                let now = std::time::Instant::now();

                if prev_misses > 0 {
                    tracing::info!(
                        server = %server_name,
                        latency_ms,
                        "Health recovered after {prev_misses} missed pings"
                    );
                }

                snapshot_tx.send_modify(|s| {
                    s.health = HealthStatus::Healthy {
                        latency_ms,
                        last_checked: now,
                    };
                });
            }
            Err(err) => {
                consecutive_misses += 1;
                tracing::warn!(
                    server = %server_name,
                    consecutive_misses,
                    "Ping failed: {err}"
                );

                let current_health = snapshot_tx.borrow().health.clone();
                let new_health = compute_health_status(consecutive_misses, &current_health);

                // Log health state transitions.
                if !matches!(current_health, HealthStatus::Degraded { .. })
                    && matches!(new_health, HealthStatus::Degraded { .. })
                {
                    tracing::info!(
                        server = %server_name,
                        consecutive_misses,
                        "Health transitioned to Degraded"
                    );
                } else if !matches!(current_health, HealthStatus::Failed { .. })
                    && matches!(new_health, HealthStatus::Failed { .. })
                {
                    tracing::info!(
                        server = %server_name,
                        consecutive_misses,
                        "Health transitioned to Failed"
                    );
                }

                snapshot_tx.send_modify(|s| {
                    s.health = new_health;
                });
            }
        }
    }
}
