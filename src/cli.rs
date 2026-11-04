use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// PM2 for MCP servers — manage, monitor, and configure your MCP servers
#[derive(Debug, Parser)]
#[command(
    name = "mcp-hub",
    version,
    about = "PM2 for MCP servers — manage, monitor, and configure your MCP servers"
)]
pub struct Cli {
    /// Disable colored output.
    #[arg(long, global = true, env = "NO_COLOR")]
    pub no_color: bool,

    /// Increase verbosity (-v for verbose, -vv for debug).
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Path to a config file (overrides default search).
    #[arg(long, short = 'c', global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start all configured MCP servers.
    Start {
        /// Run as a background daemon.
        #[arg(long)]
        daemon: bool,

        /// Where the daemon writes its log (stdout/stderr and managed servers'
        /// stderr). Defaults to `daemon.log` next to the control socket.
        /// Only used with --daemon.
        #[arg(long, value_name = "PATH", env = "MCP_HUB_LOG_FILE")]
        log_file: Option<PathBuf>,
    },

    /// Stop all running servers.
    Stop,

    /// Restart a specific server by name.
    Restart(RestartArgs),

    /// Show status of all servers (name, state, health, PID, uptime, restarts).
    Status {
        /// Print the daemon's raw status response as JSON (for scripts).
        #[arg(long)]
        json: bool,
    },

    /// Show server logs.
    Logs(LogsArgs),

    /// Reload configuration from disk without restarting the daemon.
    ///
    /// Equivalent to sending SIGHUP to the daemon process.
    Reload,

    /// Generate a Claude Code, Cursor, Codex CLI or Gemini CLI config snippet from the hub's server list.
    GenConfig(GenConfigArgs),

    /// Interactively add a new MCP server to ./mcp-hub.toml.
    Init,
}

/// Client config formats supported by `gen-config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GenConfigFormat {
    /// Claude Code `mcpServers` JSON (.mcp.json / ~/.claude.json).
    Claude,
    /// Cursor `mcpServers` JSON (~/.cursor/mcp.json).
    Cursor,
    /// Codex CLI `[mcp_servers.<name>]` TOML (~/.codex/config.toml).
    Codex,
    /// Gemini CLI `mcpServers` JSON (~/.gemini/settings.json).
    Gemini,
}

/// Arguments for the `gen-config` subcommand.
#[derive(Debug, clap::Args)]
#[command(name = "gen-config")]
pub struct GenConfigArgs {
    /// Output format.
    #[arg(long, short = 'f', value_enum, value_name = "FORMAT")]
    pub format: GenConfigFormat,

    /// Connect to the running daemon and include introspected tool/resource/prompt info as comments.
    #[arg(long)]
    pub live: bool,
}

/// Arguments for the `restart` subcommand.
#[derive(Debug, clap::Args)]
pub struct RestartArgs {
    /// Name of the server to restart.
    pub name: String,
}

/// Arguments for the `logs` subcommand.
#[derive(Debug, clap::Args)]
pub struct LogsArgs {
    /// Follow log output: print recent lines, then stream new ones (requires a running daemon).
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Filter to a specific server by name.
    #[arg(long, short = 's', value_name = "NAME")]
    pub server: Option<String>,

    /// Number of recent lines to show (default: 100).
    #[arg(long, short = 'n', default_value = "100")]
    pub lines: usize,
}
