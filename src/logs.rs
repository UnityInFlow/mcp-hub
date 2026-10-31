#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::SystemTime;

use owo_colors::AnsiColors;
use tokio::sync::{broadcast, Mutex};

// ─────────────────────────────────────────────────────────────────────────────
// Core types
// ─────────────────────────────────────────────────────────────────────────────

/// A single log line emitted by a managed MCP server.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// The name of the server that produced this line.
    pub server: String,
    /// Wall-clock time when the line was captured.
    pub timestamp: SystemTime,
    /// The raw log message.
    pub message: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-server ring buffer
// ─────────────────────────────────────────────────────────────────────────────

/// A bounded ring buffer of `LogLine`s for a single MCP server.
///
/// When the buffer is full, the oldest line is evicted to make room for the new
/// one (LOG-03: ring buffer semantics).
pub struct LogBuffer {
    lines: Mutex<VecDeque<LogLine>>,
    capacity: usize,
}

impl LogBuffer {
    /// Create a new `LogBuffer` with the given maximum `capacity`.
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Append a log line, evicting the oldest entry if the buffer is full.
    pub async fn push(&self, line: LogLine) {
        let mut guard = self.lines.lock().await;
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(line);
    }

    /// Return a clone of all buffered lines in insertion (FIFO) order.
    pub async fn snapshot(&self) -> Vec<LogLine> {
        self.lines.lock().await.iter().cloned().collect()
    }

    /// Return a clone of the last `n` buffered lines.
    ///
    /// If fewer than `n` lines exist, all lines are returned.
    pub async fn snapshot_last(&self, n: usize) -> Vec<LogLine> {
        let guard = self.lines.lock().await;
        let len = guard.len();
        let start = len.saturating_sub(n);
        guard.iter().skip(start).cloned().collect()
    }

    /// Return the current number of buffered lines.
    pub async fn len(&self) -> usize {
        self.lines.lock().await.len()
    }

    /// Return `true` if the buffer contains no lines.
    pub async fn is_empty(&self) -> bool {
        self.lines.lock().await.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-server log aggregator
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregates log lines from all managed MCP servers.
///
/// Each server gets its own `LogBuffer` ring buffer (LOG-03). A broadcast
/// channel carries every line to any live subscriber (LOG-04, LOG-05).
pub struct LogAggregator {
    /// Per-server buffers. Guarded by a std mutex that is never held across an
    /// `.await`; buffers are created on demand so servers added via reload are
    /// captured, and removed when a server is dropped.
    buffers: StdMutex<HashMap<String, Arc<LogBuffer>>>,
    capacity_per_server: usize,
    all_tx: broadcast::Sender<LogLine>,
}

impl LogAggregator {
    /// Create a new `LogAggregator`, pre-registering one `LogBuffer` per server name.
    ///
    /// Servers not listed here (for example those added later by a config
    /// reload) get a buffer of the same capacity on their first `push` or via
    /// [`LogAggregator::ensure_buffer`].
    pub fn new(server_names: &[String], capacity_per_server: usize) -> Self {
        let buffers = server_names
            .iter()
            .map(|name| (name.clone(), Arc::new(LogBuffer::new(capacity_per_server))))
            .collect();

        let (all_tx, _) = broadcast::channel(1024);

        Self {
            buffers: StdMutex::new(buffers),
            capacity_per_server,
            all_tx,
        }
    }

    /// Lock the buffer map, recovering from poisoning (the map stays valid).
    fn lock_buffers(&self) -> MutexGuard<'_, HashMap<String, Arc<LogBuffer>>> {
        self.buffers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Return the buffer for `server`, creating it if it does not exist yet.
    pub fn ensure_buffer(&self, server: &str) -> Arc<LogBuffer> {
        let mut guard = self.lock_buffers();
        Arc::clone(
            guard
                .entry(server.to_string())
                .or_insert_with(|| Arc::new(LogBuffer::new(self.capacity_per_server))),
        )
    }

    /// Drop the buffer for `server` (e.g. when a reload removes the server).
    ///
    /// Returns `true` if a buffer was present.
    pub fn remove_server(&self, server: &str) -> bool {
        self.lock_buffers().remove(server).is_some()
    }

    /// Push a log message for `server`, stamping it with the current time.
    ///
    /// A buffer is created on demand for unknown server names. Also broadcasts
    /// the line to all active subscribers. Lagged subscriber errors are
    /// silently ignored.
    pub async fn push(&self, server: &str, message: String) {
        let line = LogLine {
            server: server.to_string(),
            timestamp: SystemTime::now(),
            message,
        };

        let buf = self.ensure_buffer(server);
        buf.push(line.clone()).await;

        // Broadcast — ignore errors (no subscribers, or lagged).
        let _ = self.all_tx.send(line);
    }

    /// Return the `LogBuffer` for the given server name, if registered.
    pub fn get_buffer(&self, server: &str) -> Option<Arc<LogBuffer>> {
        self.lock_buffers().get(server).cloned()
    }

    /// Subscribe to the all-server broadcast stream.
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.all_tx.subscribe()
    }

    /// Merge all server buffers into a single `Vec<LogLine>` sorted by timestamp.
    pub async fn snapshot_all(&self) -> Vec<LogLine> {
        let bufs: Vec<Arc<LogBuffer>> = self.lock_buffers().values().cloned().collect();
        let mut all: Vec<LogLine> = Vec::new();
        for buf in bufs {
            all.extend(buf.snapshot().await);
        }
        all.sort_by_key(|l| l.timestamp);
        all
    }

    /// Return the list of registered server names.
    pub fn server_names(&self) -> Vec<String> {
        self.lock_buffers().keys().cloned().collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatting helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Format a `LogLine` for terminal display.
///
/// Output: `"server-name | 2026-04-02T10:15:30Z message"`
///
/// When `color` is `true`, the server name prefix is colored using a
/// deterministic hash-based color assignment (so the same server always gets
/// the same color across runs).
pub fn format_log_line(line: &LogLine, color: bool) -> String {
    let ts = format_system_time(line.timestamp);

    if color {
        use owo_colors::OwoColorize as _;
        let color = server_color(&line.server);
        let prefix = line.server.color(color).to_string();
        format!("{prefix} | {ts} {}", line.message)
    } else {
        format!("{} | {ts} {}", line.server, line.message)
    }
}

/// Pick a terminal color for a server name using a stable hash.
fn server_color(name: &str) -> AnsiColors {
    const PALETTE: [AnsiColors; 6] = [
        AnsiColors::Cyan,
        AnsiColors::Green,
        AnsiColors::Yellow,
        AnsiColors::Magenta,
        AnsiColors::Blue,
        AnsiColors::BrightCyan,
    ];

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % PALETTE.len();
    PALETTE[idx]
}

/// Format a `SystemTime` as `YYYY-MM-DDTHH:MM:SSZ` (RFC 3339, second precision).
///
/// Uses manual arithmetic on `duration_since(UNIX_EPOCH)` — no chrono dependency.
pub(crate) fn format_system_time(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Civil time from Unix epoch — proleptic Gregorian calendar.
    let mut days = (secs / 86400) as u32;
    let time_secs = secs % 86400;

    let hour = time_secs / 3600;
    let minute = (time_secs % 3600) / 60;
    let second = time_secs % 60;

    // Compute year, month, day using the civil calendar algorithm.
    // Reference: https://howardhinnant.github.io/date_algorithms.html
    let z = days as i32 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    // Suppress the unused mut warning — `days` is consumed above.
    let _ = &mut days;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
