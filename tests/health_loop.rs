//! End-to-end health loop tests with fake MCP servers (bash): the real
//! `run_health_check_loop` pinging through the real dispatcher.
//!
//! Complements `health_monitor.rs`, which covers a single healthy ping.

#![cfg(unix)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcp_hub::mcp::dispatcher::{reader_task, IdAllocator, PendingMap, SharedStdin};
use mcp_hub::mcp::health::run_health_check_loop;
use mcp_hub::types::{HealthStatus, ServerSnapshot};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Answers every request with an empty result (like a healthy MCP server).
const RESPONDER_SCRIPT: &str = r#"
while IFS= read -r line; do
    id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
    echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
done
"#;

/// Reads requests but never answers any of them (a wedged server).
const SILENT_SCRIPT: &str = "cat > /dev/null";

/// Spawn a fake server. `kill_on_drop` guarantees cleanup even when an
/// assertion fails before the explicit kill.
fn spawn_fake(script: &str) -> (tokio::process::Child, SharedStdin, PendingMap) {
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("bash should be available");
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let stdin_shared: SharedStdin = Arc::new(Mutex::new(stdin));
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (p, s) = (Arc::clone(&pending), Arc::clone(&stdin_shared));
    tokio::spawn(async move { reader_task(stdout, p, s).await });
    (child, stdin_shared, pending)
}

/// Misses are only counted after the 5 s ping timeout, so reaching Failed
/// (7 misses) takes ~35 s of wall clock. Virtual time (`start_paused`) makes
/// the real loop, dispatcher timeout and state machine run instantly; the
/// silent server never produces I/O that could race the virtual clock.
#[tokio::test(start_paused = true)]
async fn missed_pings_drive_unknown_to_degraded_to_failed() {
    let (mut child, stdin, pending) = spawn_fake(SILENT_SCRIPT);
    let (tx, mut rx) = tokio::sync::watch::channel(ServerSnapshot::default());
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run_health_check_loop(
        "silent".to_string(),
        1,
        stdin,
        pending,
        Arc::new(IdAllocator::new()),
        tx,
        cancel.clone(),
    ));

    // Record the miss count of every distinct health state, in order.
    let mut seen: Vec<(&'static str, u32)> = Vec::new();
    let observed = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let step = match rx.borrow_and_update().health.clone() {
                HealthStatus::Degraded {
                    consecutive_misses, ..
                } => Some(("degraded", consecutive_misses)),
                HealthStatus::Failed { consecutive_misses } => Some(("failed", consecutive_misses)),
                HealthStatus::Healthy { .. } | HealthStatus::Unknown => None,
            };
            if let Some(step) = step {
                if seen.last() != Some(&step) {
                    seen.push(step);
                }
                if step.0 == "failed" {
                    return;
                }
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    })
    .await;

    cancel.cancel();
    let _ = task.await;
    let _ = child.kill().await;

    assert!(observed.is_ok(), "never reached Failed; saw {seen:?}");
    let first_degraded = seen.iter().position(|(k, _)| *k == "degraded");
    let first_failed = seen.iter().position(|(k, _)| *k == "failed");
    assert!(
        first_degraded.is_some() && first_degraded < first_failed,
        "expected Degraded before Failed, saw {seen:?}"
    );
    assert_eq!(seen.first(), Some(&("degraded", 2)), "saw {seen:?}");
    assert_eq!(seen.last(), Some(&("failed", 7)), "saw {seen:?}");
}

/// With a 1 s interval a responsive server must be pinged about once a second
/// (not every 30 s default): several pings in a few seconds, evenly spaced.
#[tokio::test]
async fn configured_interval_is_honoured() {
    let (mut child, stdin, pending) = spawn_fake(RESPONDER_SCRIPT);
    let (tx, mut rx) = tokio::sync::watch::channel(ServerSnapshot::default());
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run_health_check_loop(
        "responsive".to_string(),
        1,
        stdin,
        pending,
        Arc::new(IdAllocator::new()),
        tx,
        cancel.clone(),
    ));

    // Each successful ping stamps a fresh `last_checked`; collect them.
    let mut stamps: Vec<Instant> = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(3600), async {
        loop {
            if let HealthStatus::Healthy { last_checked, .. } =
                rx.borrow_and_update().health.clone()
            {
                if stamps.last() != Some(&last_checked) {
                    stamps.push(last_checked);
                }
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    })
    .await;

    cancel.cancel();
    let _ = task.await;
    let _ = child.kill().await;

    assert!(
        stamps.len() >= 3,
        "expected >= 3 pings in 3.6 s with a 1 s interval, got {}",
        stamps.len()
    );
    for pair in stamps.windows(2) {
        let gap = pair[1].duration_since(pair[0]);
        assert!(
            gap >= Duration::from_millis(500) && gap <= Duration::from_millis(2000),
            "ping gap {gap:?} is not roughly the 1 s interval"
        );
    }
}
