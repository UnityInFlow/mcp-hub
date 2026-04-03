mod cli;
mod config;
mod types;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    // Subcommand dispatch will be added in Plan 03
    println!("mcp-hub v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
