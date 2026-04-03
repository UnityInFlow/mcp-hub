//! MCP health check loop — sends JSON-RPC pings over stdin/stdout.
//!
//! This module is implemented in Task 4 (02-02-04). The stub constants and
//! function signatures are declared here so supervisor.rs can reference them
//! during the Task 3 watch-channel upgrade (02-02-03).

use std::time::Duration;

use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio_util::sync::CancellationToken;

use crate::mcp::protocol::{JsonRpcResponse, PingRequest};
use crate::types::{compute_health_status, HealthStatus, ServerSnapshot};

/// Default health check interval in seconds (configurable per server via config).
pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Send a single MCP JSON-RPC ping to the server and return latency in milliseconds.
///
/// Writes `PingRequest::new(id)` as newline-delimited JSON to `stdin`, then reads
/// from `stdout` until a matching response arrives. Non-matching lines (unsolicited
/// server output) are drained via `tracing::debug!` to prevent pipe backpressure
/// (PITFALL #2). The entire operation is wrapped in a 5-second timeout (HLTH-04).
pub async fn ping_server(
    stdin: &mut ChildStdin,
    stdout_reader: &mut tokio::io::Lines<BufReader<ChildStdout>>,
    id: u64,
) -> anyhow::Result<u64> {
    let request = PingRequest::new(id);
    let mut json = serde_json::to_string(&request)?;
    json.push('\n');

    let start = std::time::Instant::now();

    stdin.write_all(json.as_bytes()).await?;
    stdin.flush().await?;

    // Read lines within the 5-second timeout, draining non-matching output.
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        // Allow up to 100 non-matching lines before giving up (prevents infinite drain).
        for _ in 0..100u32 {
            let line = stdout_reader
                .next_line()
                .await
                .map_err(|e| anyhow!("IO error reading stdout: {e}"))?
                .ok_or_else(|| anyhow!("stdout closed unexpectedly"))?;

            match serde_json::from_str::<JsonRpcResponse>(&line) {
                Ok(resp) if resp.id == id => {
                    if resp.error.is_some() {
                        return Err(anyhow!(
                            "Server returned error response to ping id={id}: {:?}",
                            resp.error
                        ));
                    }
                    return Ok(());
                }
                Ok(resp) => {
                    // Unsolicited response — drain and continue.
                    tracing::debug!("Received out-of-order response id={}, expected id={id}", resp.id);
                }
                Err(_) => {
                    // Non-JSON or notification line — drain and continue.
                    tracing::debug!("Draining non-ping stdout line");
                }
            }
        }
        Err(anyhow!("Too many non-matching lines from server during ping id={id}"))
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(start.elapsed().as_millis() as u64),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(anyhow!("Ping id={id} timed out after 5s")),
    }
}

/// Run the per-server health check loop.
///
/// - Sends MCP JSON-RPC pings at `interval_secs` intervals.
/// - Updates `snapshot_tx` with `HealthStatus::Healthy` on success, or transitions
///   through Degraded/Failed on consecutive misses via `compute_health_status`.
/// - Exits cleanly when `cancel` is triggered.
///
/// The health task owns `stdin` and `stdout` for the lifetime of the spawned process.
/// The supervisor cancels the token before killing the process, and again on restart.
pub async fn run_health_check_loop(
    server_name: String,
    interval_secs: u64,
    stdin: ChildStdin,
    stdout: ChildStdout,
    snapshot_tx: tokio::sync::watch::Sender<ServerSnapshot>,
    cancel: CancellationToken,
) {
    use tokio::time::MissedTickBehavior;

    let mut request_id: u64 = 1;
    let mut consecutive_misses: u32 = 0;

    let mut stdin = stdin;
    let stdout_reader = BufReader::new(stdout);
    let mut lines = stdout_reader.lines();

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

        match ping_server(&mut stdin, &mut lines, request_id).await {
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

        request_id += 1;
    }
}
