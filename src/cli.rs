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
    Start,

    /// Stop all running servers.
    Stop,

    /// Restart a specific server by name.
    Restart(RestartArgs),
}

/// Arguments for the `restart` subcommand.
#[derive(Debug, clap::Args)]
pub struct RestartArgs {
    /// Name of the server to restart.
    pub name: String,
}
