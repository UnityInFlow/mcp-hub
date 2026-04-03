#![allow(dead_code)]

use std::fmt;
use std::time::{Duration, Instant};

/// The lifecycle state of a managed MCP server process.
///
/// `Instant` is not `PartialEq`/`Eq`, so those traits are intentionally omitted.
#[derive(Debug, Clone)]
pub enum ProcessState {
    /// The server is not running.
    Stopped,
    /// The server is starting up.
    Starting,
    /// The server is running normally.
    Running,
    /// The server is waiting before the next restart attempt.
    Backoff {
        attempt: u32,
        until: std::time::Instant,
    },
    /// The server has exceeded `max_attempts` and will not be restarted.
    Fatal,
    /// The server is in the process of being stopped.
    Stopping,
}

impl fmt::Display for ProcessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessState::Stopped => write!(f, "stopped"),
            ProcessState::Starting => write!(f, "starting"),
            ProcessState::Running => write!(f, "running"),
            ProcessState::Backoff { attempt, .. } => write!(f, "backoff ({})", attempt),
            ProcessState::Fatal => write!(f, "fatal"),
            ProcessState::Stopping => write!(f, "stopping"),
        }
    }
}

/// Configuration for exponential backoff on server crashes.
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Base delay in seconds before the first retry.
    pub base_delay_secs: f64,
    /// Maximum delay in seconds between retries.
    pub max_delay_secs: f64,
    /// Jitter factor applied to each delay (0.0–1.0).
    pub jitter_factor: f64,
    /// Number of consecutive failures before the server is marked `Fatal`.
    pub max_attempts: u32,
    /// Seconds the server must remain `Running` before the failure counter resets.
    pub stable_window_secs: u64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            base_delay_secs: 1.0,
            max_delay_secs: 60.0,
            jitter_factor: 0.3,
            max_attempts: 10,
            stable_window_secs: 60,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health monitoring types (Phase 2)
// ─────────────────────────────────────────────────────────────────────────────

/// The health state of a managed MCP server, derived from ping results.
///
/// State transitions are driven by `compute_health_status`.
/// `Instant` fields are not serializable — they remain in-memory only.
#[derive(Debug, Clone, Default)]
pub enum HealthStatus {
    /// No ping has been attempted yet (initial state and after restart).
    #[default]
    Unknown,
    /// The server responded to the most recent ping.
    Healthy {
        latency_ms: u64,
        last_checked: Instant,
    },
    /// The server has missed 2–6 consecutive pings.
    Degraded {
        consecutive_misses: u32,
        last_success: Option<Instant>,
    },
    /// The server has missed 7 or more consecutive pings.
    Failed { consecutive_misses: u32 },
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthStatus::Unknown => write!(f, "unknown"),
            HealthStatus::Healthy { .. } => write!(f, "healthy"),
            HealthStatus::Degraded {
                consecutive_misses, ..
            } => write!(f, "degraded ({consecutive_misses} missed)"),
            HealthStatus::Failed {
                consecutive_misses, ..
            } => write!(f, "failed ({consecutive_misses} missed)"),
        }
    }
}

/// A point-in-time snapshot of a managed server's state, health, and metadata.
///
/// Replaces the `(ProcessState, Option<u32>)` watch channel payload in Phase 2.
/// The `uptime_since` field uses `Instant` (monotonic) and is not serializable.
#[derive(Debug, Clone)]
pub struct ServerSnapshot {
    pub process_state: ProcessState,
    pub health: HealthStatus,
    pub pid: Option<u32>,
    pub uptime_since: Option<Instant>,
    pub restart_count: u32,
    pub transport: String,
}

impl Default for ServerSnapshot {
    fn default() -> Self {
        Self {
            process_state: ProcessState::Stopped,
            health: HealthStatus::Unknown,
            pid: None,
            uptime_since: None,
            restart_count: 0,
            transport: "stdio".to_string(),
        }
    }
}

/// Compute the next `HealthStatus` after a ping failure.
///
/// This function is only called on the **failure** path. The success path
/// directly constructs `HealthStatus::Healthy { latency_ms, last_checked }`.
///
/// Transition thresholds (D-02, D-03):
/// - 1 miss: stay in current state (no change yet)
/// - 2–6 consecutive misses from Healthy/Unknown: -> `Degraded`
/// - 2–6 consecutive misses already Degraded: stay `Degraded` with updated count
/// - 7+ consecutive misses: -> `Failed`
/// - Already `Failed`: stay `Failed` with updated count
pub fn compute_health_status(consecutive_misses: u32, current: &HealthStatus) -> HealthStatus {
    if consecutive_misses >= 7 {
        return HealthStatus::Failed { consecutive_misses };
    }

    if consecutive_misses < 2 {
        // 1 miss — stay in current state unchanged.
        return current.clone();
    }

    // 2–6 misses.
    match current {
        HealthStatus::Failed { .. } => HealthStatus::Failed { consecutive_misses },
        HealthStatus::Degraded { last_success, .. } => HealthStatus::Degraded {
            consecutive_misses,
            last_success: *last_success,
        },
        HealthStatus::Healthy { last_checked, .. } => HealthStatus::Degraded {
            consecutive_misses,
            last_success: Some(*last_checked),
        },
        HealthStatus::Unknown => HealthStatus::Degraded {
            consecutive_misses,
            last_success: None,
        },
    }
}

/// Format a `Duration` as `"HH:MM:SS"`.
///
/// Hours can exceed 24 (e.g. `"25:01:01"` for 90 061 seconds).
pub fn format_uptime(elapsed: Duration) -> String {
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
