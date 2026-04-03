#![allow(dead_code)]

use std::fmt;

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
