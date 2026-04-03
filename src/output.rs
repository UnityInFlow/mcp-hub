use std::io::IsTerminal as _;

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

/// Format the status table for the given list of servers and return it as a `String`.
///
/// Columns: Name, State, Health, PID, Uptime, Restarts, Transport (D-09).
pub fn format_status_table(servers: &[(String, ServerSnapshot)], color: bool) -> String {
    let mut table = Table::new();
    table.set_header(vec![
        "Name",
        "State",
        "Health",
        "PID",
        "Uptime",
        "Restarts",
        "Transport",
    ]);

    for (name, snapshot) in servers {
        let state = &snapshot.process_state;
        let state_str = state.to_string();

        let state_cell = if color {
            let cell_color = match state {
                ProcessState::Running => Some(Color::Green),
                ProcessState::Starting => Some(Color::Yellow),
                ProcessState::Backoff { .. } => Some(Color::Yellow),
                ProcessState::Stopping => Some(Color::Yellow),
                ProcessState::Fatal => Some(Color::Red),
                ProcessState::Stopped => Some(Color::DarkGrey),
            };
            match cell_color {
                Some(c) => Cell::new(&state_str).fg(c),
                None => Cell::new(&state_str),
            }
        } else {
            Cell::new(&state_str)
        };

        let health_str = snapshot.health.to_string();
        let health_cell = if color {
            let cell_color = match &snapshot.health {
                HealthStatus::Healthy { .. } => Some(Color::Green),
                HealthStatus::Degraded { .. } => Some(Color::Yellow),
                HealthStatus::Failed { .. } => Some(Color::Red),
                HealthStatus::Unknown => Some(Color::DarkGrey),
            };
            match cell_color {
                Some(c) => Cell::new(&health_str).fg(c),
                None => Cell::new(&health_str),
            }
        } else {
            Cell::new(&health_str)
        };

        let pid_str = snapshot
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());

        let uptime_str = snapshot
            .uptime_since
            .map(|since| format_uptime(since.elapsed()))
            .unwrap_or_else(|| "-".to_string());

        let restarts_str = snapshot.restart_count.to_string();

        table.add_row(vec![
            Cell::new(name),
            state_cell,
            health_cell,
            Cell::new(&pid_str),
            Cell::new(&uptime_str),
            Cell::new(&restarts_str),
            Cell::new(&snapshot.transport),
        ]);
    }

    table.to_string()
}

/// Print a formatted status table for the given list of servers.
///
/// Columns: Name, State, Health, PID, Uptime, Restarts, Transport (D-09).
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
        .init();
}
