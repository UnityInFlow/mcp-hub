use std::io::IsTerminal as _;
use std::time::Duration;

use comfy_table::{Cell, Color, Table};
use tracing_subscriber::EnvFilter;

use crate::supervisor::ServerHandle;
use crate::types::{format_uptime, HealthStatus, ProcessState, ServerSnapshot};

/// Return `true` if colored output should be used.
///
/// Colors are disabled when:
/// - the `--no-color` flag is set, OR
/// - stdout is not a TTY (e.g. piped output).
pub fn use_colors(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// One rendered row of the status table, independent of where the data came
/// from (a live `ServerSnapshot` in foreground mode, the daemon's `Status`
/// response in daemon mode).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRow {
    pub name: String,
    pub state: String,
    pub state_color: Option<Color>,
    pub health: String,
    pub health_color: Option<Color>,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub restarts: u64,
    pub transport: String,
    /// `(tools, resources, prompts)`; `None` until the server was introspected.
    pub capabilities: Option<(usize, usize, usize)>,
}

impl StatusRow {
    fn from_snapshot(name: &str, snapshot: &ServerSnapshot) -> Self {
        let state_color = match &snapshot.process_state {
            ProcessState::Running => Color::Green,
            ProcessState::Starting => Color::Yellow,
            ProcessState::Backoff { .. } => Color::Yellow,
            ProcessState::Stopping => Color::Yellow,
            ProcessState::Fatal => Color::Red,
            ProcessState::Stopped => Color::DarkGrey,
        };
        let health_color = match &snapshot.health {
            HealthStatus::Healthy { .. } => Color::Green,
            HealthStatus::Degraded { .. } => Color::Yellow,
            HealthStatus::Failed { .. } => Color::Red,
            HealthStatus::Unknown | HealthStatus::NotChecked => Color::DarkGrey,
        };
        let caps = &snapshot.capabilities;
        Self {
            name: name.to_string(),
            state: snapshot.process_state.to_string(),
            state_color: Some(state_color),
            health: snapshot.health.to_string(),
            health_color: Some(health_color),
            pid: snapshot.pid,
            uptime_secs: snapshot.uptime_since.map(|since| since.elapsed().as_secs()),
            restarts: u64::from(snapshot.restart_count),
            transport: snapshot.transport.clone(),
            capabilities: caps
                .introspected_at
                .map(|_| (caps.tools.len(), caps.resources.len(), caps.prompts.len())),
        }
    }

    /// Build a row from one entry of the daemon's `Status` response.
    ///
    /// Missing or mistyped fields degrade to placeholders instead of failing,
    /// so a newer or older daemon still renders; only a missing `name` is an error.
    fn from_status_json(entry: &serde_json::Value) -> anyhow::Result<Self> {
        let name = entry["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Status entry is missing its 'name' field"))?;
        let state = entry["state"].as_str().unwrap_or("unknown");
        let health = entry["health"].as_str().unwrap_or("unknown");
        let count = |key: &str| {
            entry[key]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        let introspected = entry["introspected"].as_bool().unwrap_or(false);
        Ok(Self {
            name: name.to_string(),
            state: state.to_string(),
            state_color: Some(state_color_for(state)),
            health: health.to_string(),
            health_color: Some(health_color_for(health)),
            pid: entry["pid"].as_u64().and_then(|p| u32::try_from(p).ok()),
            uptime_secs: entry["uptime_secs"].as_u64(),
            restarts: entry["restart_count"].as_u64().unwrap_or(0),
            transport: entry["transport"].as_str().unwrap_or("-").to_string(),
            capabilities: introspected
                .then(|| (count("tools"), count("resources"), count("prompts"))),
        })
    }
}

/// Colour for a rendered state string (mirrors the `ProcessState` mapping).
fn state_color_for(state: &str) -> Color {
    match state {
        "running" => Color::Green,
        "fatal" => Color::Red,
        "stopped" => Color::DarkGrey,
        // "starting", "stopping" and "backoff (N)" are all transient.
        _ => Color::Yellow,
    }
}

/// Colour for a rendered health string (mirrors the `HealthStatus` mapping).
fn health_color_for(health: &str) -> Color {
    if health == "healthy" {
        Color::Green
    } else if health.starts_with("degraded") {
        Color::Yellow
    } else if health.starts_with("failed") {
        Color::Red
    } else {
        // "unknown" and anything a different daemon version might send.
        Color::DarkGrey
    }
}

fn colored_cell(text: &str, color: Option<Color>, enabled: bool) -> Cell {
    match color {
        Some(c) if enabled => Cell::new(text).fg(c),
        Some(_) | None => Cell::new(text),
    }
}

/// Render pre-built rows as the status table.
///
/// Columns: Name, State, Health, PID, Uptime, Restarts, Transport, Tools (D-09).
pub fn format_status_rows(rows: &[StatusRow], color: bool) -> String {
    let mut table = Table::new();
    table.set_header(vec![
        "Name",
        "State",
        "Health",
        "PID",
        "Uptime",
        "Restarts",
        "Transport",
        "Tools",
    ]);

    for row in rows {
        let pid_str = row
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        let uptime_str = row
            .uptime_secs
            .map(|secs| format_uptime(Duration::from_secs(secs)))
            .unwrap_or_else(|| "-".to_string());
        let tools_str = match row.capabilities {
            Some((tools, resources, prompts)) => format!("{tools}T/{resources}R/{prompts}P"),
            None => "-".to_string(),
        };

        table.add_row(vec![
            Cell::new(&row.name),
            colored_cell(&row.state, row.state_color, color),
            colored_cell(&row.health, row.health_color, color),
            Cell::new(&pid_str),
            Cell::new(&uptime_str),
            Cell::new(row.restarts.to_string()),
            Cell::new(&row.transport),
            Cell::new(&tools_str),
        ]);
    }

    table.to_string()
}

/// Format the status table for the given list of servers and return it as a `String`.
///
/// Columns: Name, State, Health, PID, Uptime, Restarts, Transport, Tools (D-09).
pub fn format_status_table(servers: &[(String, ServerSnapshot)], color: bool) -> String {
    let rows: Vec<StatusRow> = servers
        .iter()
        .map(|(name, snapshot)| StatusRow::from_snapshot(name, snapshot))
        .collect();
    format_status_rows(&rows, color)
}

/// Format the daemon's `Status` response data (a JSON array) as the status table.
pub fn format_status_json(data: &serde_json::Value, color: bool) -> anyhow::Result<String> {
    let entries = data
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected an array from the daemon status response"))?;
    let rows = entries
        .iter()
        .map(StatusRow::from_status_json)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(format_status_rows(&rows, color))
}

/// Print a formatted status table for the given list of servers.
///
/// Columns: Name, State, Health, PID, Uptime, Restarts, Transport, Tools (D-09).
/// This is a thin wrapper around [`format_status_table`] for easy testing.
pub fn print_status_table(servers: &[(String, ServerSnapshot)], color: bool) {
    println!("{}", format_status_table(servers, color));
}

/// Collect current state snapshots from all server handles.
///
/// This is a non-blocking operation — `watch::Receiver::borrow` returns immediately.
pub fn collect_states_from_handles(handles: &[ServerHandle]) -> Vec<(String, ServerSnapshot)> {
    handles
        .iter()
        .map(|handle| {
            let snapshot = handle.state_rx.borrow().clone();
            (handle.name.clone(), snapshot)
        })
        .collect()
}

/// Configure the global tracing subscriber based on the verbosity level.
///
/// - `0` (default): `warn` — quiet by default (D-17)
/// - `1` (`-v`): `info` — show start/stop events
/// - `2+` (`-vv`): `debug` — show spawn details
pub fn configure_tracing(verbose: u8) {
    let filter = match verbose {
        0 => EnvFilter::new("warn"),
        1 => EnvFilter::new("info"),
        _ => EnvFilter::new("debug"),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        // Escape codes would litter the daemon log file; only colour a terminal.
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}
