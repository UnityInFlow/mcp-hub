/// Tests for the `mcp-hub logs` and `mcp-hub status` CLI subcommands (Plan 02-03, Task 6).
///
/// CLI tests use assert_cmd to run the binary as a subprocess and verify exit codes
/// and stderr messages. Unit tests verify LogsArgs flag parsing.
use assert_cmd::Command;
use clap::Parser;
use mcp_hub::cli::{Cli, Commands};

// ─────────────────────────────────────────────────────────────────────────────
// CLI subprocess tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn logs_subcommand_prints_daemon_required() {
    let mut cmd = Command::cargo_bin("mcp-hub").expect("Failed to find mcp-hub binary");
    cmd.arg("logs");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("no daemon running"));
}

#[test]
fn logs_follow_subcommand_prints_daemon_required() {
    let mut cmd = Command::cargo_bin("mcp-hub").expect("Failed to find mcp-hub binary");
    cmd.args(["logs", "--follow"]);
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("requires daemon mode"));
}

#[test]
fn logs_subcommand_with_server_filter() {
    let mut cmd = Command::cargo_bin("mcp-hub").expect("Failed to find mcp-hub binary");
    cmd.args(["logs", "--server", "foo"]);
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("no daemon running"));
}

#[test]
fn status_subcommand_prints_daemon_required() {
    let mut cmd = Command::cargo_bin("mcp-hub").expect("Failed to find mcp-hub binary");
    cmd.arg("status");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("no daemon running"));
}

// ─────────────────────────────────────────────────────────────────────────────
// LogsArgs unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn logs_args_parsing() {
    let cli = Cli::try_parse_from([
        "mcp-hub",
        "logs",
        "-f",
        "-s",
        "my-server",
        "-n",
        "50",
    ])
    .expect("Failed to parse CLI args");

    match cli.command {
        Commands::Logs(args) => {
            assert!(args.follow, "follow must be true");
            assert_eq!(
                args.server,
                Some("my-server".to_string()),
                "server must be Some(\"my-server\")"
            );
            assert_eq!(args.lines, 50, "lines must be 50");
        }
        other => panic!("Expected Commands::Logs, got {other:?}"),
    }
}

#[test]
fn logs_args_defaults() {
    let cli = Cli::try_parse_from(["mcp-hub", "logs"]).expect("Failed to parse CLI args");

    match cli.command {
        Commands::Logs(args) => {
            assert!(!args.follow, "follow must default to false");
            assert_eq!(args.server, None, "server must default to None");
            assert_eq!(args.lines, 100, "lines must default to 100");
        }
        other => panic!("Expected Commands::Logs, got {other:?}"),
    }
}

#[test]
fn logs_args_long_flags() {
    let cli = Cli::try_parse_from([
        "mcp-hub",
        "logs",
        "--follow",
        "--server",
        "backend",
        "--lines",
        "200",
    ])
    .expect("Failed to parse CLI args with long flags");

    match cli.command {
        Commands::Logs(args) => {
            assert!(args.follow, "follow must be true with --follow");
            assert_eq!(args.server, Some("backend".to_string()));
            assert_eq!(args.lines, 200);
        }
        other => panic!("Expected Commands::Logs, got {other:?}"),
    }
}
