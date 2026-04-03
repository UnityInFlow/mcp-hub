use std::io::IsTerminal as _;

use comfy_table::{Cell, Color, Table};
use tracing_subscriber::EnvFilter;

use crate::supervisor::ServerHandle;
use crate::types::ProcessState;

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

/// Print a formatted status table for the given list of servers.
///
/// Each row shows the server name, lifecycle state (optionally colored), and PID.
pub fn print_status_table(servers: &[(String, ProcessState, Option<u32>)], color: bool) {
    let mut table = Table::new();
    table.set_header(vec!["Name", "State", "PID"]);

    for (name, state, pid) in servers {
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

        let pid_str = pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());

        table.add_row(vec![Cell::new(name), state_cell, Cell::new(&pid_str)]);
    }

    println!("{table}");
}

/// Collect current state snapshots from all server handles.
///
/// This is a non-blocking operation — `watch::Receiver::borrow` returns immediately.
pub fn collect_states_from_handles(
    handles: &[ServerHandle],
) -> Vec<(String, ProcessState, Option<u32>)> {
    handles
        .iter()
        .map(|handle| {
            let (state, pid) = handle.state_rx.borrow().clone();
            (handle.name.clone(), state, pid)
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
