#[cfg(not(unix))]
compile_error!("mcp-hub supports Unix-like systems only (Linux, macOS); see https://github.com/UnityInFlow/mcp-hub/issues/30");

pub mod cli;
pub mod commands;
pub mod config;
pub mod control;
pub mod daemon;
pub mod gen_config;
pub mod init;
pub mod logs;
pub mod mcp;
pub mod output;
pub mod supervisor;
pub mod types;
pub mod web;
