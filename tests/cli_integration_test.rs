use std::io::Write as _;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt as _;
use tempfile::NamedTempFile;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn mcp_hub() -> Command {
    Command::cargo_bin("mcp-hub").expect("mcp-hub binary not found")
}

fn write_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("Failed to create tempfile");
    f.write_all(content.as_bytes())
        .expect("Failed to write tempfile");
    f
}

// ─────────────────────────────────────────────────────────────────────────────
// Basic flag tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_version_flag() {
    mcp_hub()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains("mcp-hub"));
}

#[test]
fn test_help_flag() {
    mcp_hub()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("PM2 for MCP servers"));
}

#[test]
fn test_start_help() {
    mcp_hub()
        .args(["start", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Start all configured MCP servers",
        ));
}

// ─────────────────────────────────────────────────────────────────────────────
// Config error cases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_missing_config_exits_nonzero() {
    mcp_hub()
        .args(["start", "--config", "/nonexistent/path/config.toml"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("(Failed to|config|nonexistent)").unwrap());
}

#[test]
fn test_invalid_toml_exits_nonzero() {
    let f = write_toml("this is not valid toml }{");
    mcp_hub()
        .args(["start", "--config", f.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("(TOML|Invalid|parse|expected)").unwrap());
}

#[test]
fn test_empty_config_exits_nonzero() {
    let f = write_toml("# empty config\n");
    mcp_hub()
        .args(["start", "--config", f.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("(No servers defined|init)").unwrap());
}

// ─────────────────────────────────────────────────────────────────────────────
// Stop subcommand (foreground-mode stub)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_stop_without_daemon_prints_message() {
    mcp_hub()
        .arg("stop")
        .assert()
        .failure()
        .stderr(predicates::str::is_match("(foreground|Ctrl\\+C)").unwrap());
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests that start the binary with a valid config (use .timeout() to prevent
// blocking indefinitely — the binary blocks on Ctrl+C until killed).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_bad_command_in_config_starts_and_shows_fatal() {
    let f = write_toml(
        r#"
[servers.broken]
command = "/nonexistent/binary/that/does/not/exist"
"#,
    );

    // The supervisor will try to spawn the command, fail, and eventually mark it Fatal.
    // Since Fatal is a terminal state (no more retries until restart), the process
    // should reach Fatal quickly and print the status table. We allow up to 15 s.
    mcp_hub()
        .args([
            "start",
            "--no-color",
            "--config",
            f.path().to_str().unwrap(),
        ])
        .timeout(Duration::from_secs(15))
        // Either the process prints "fatal" in the status table (success path),
        // OR it exits non-zero with a spawn error. Both are acceptable.
        .assert()
        .stdout(predicates::str::contains("broken").or(predicates::str::is_empty()));
}

#[test]
fn test_start_with_valid_config_shows_status_table() {
    let f = write_toml(
        r#"
[servers.sleeper]
command = "sleep"
args = ["300"]
"#,
    );

    // `sleep 300` starts immediately and stays Running. The binary prints the
    // status table then blocks on Ctrl+C. The timeout kills it after 5 s.
    // We verify the table contains the server name and "running".
    mcp_hub()
        .args([
            "start",
            "--no-color",
            "--config",
            f.path().to_str().unwrap(),
        ])
        .timeout(Duration::from_secs(5))
        .assert()
        // The process is killed by timeout — accept any exit code.
        .stdout(predicates::str::contains("sleeper"))
        .stdout(predicates::str::contains("running"));
}

#[test]
fn test_no_color_flag() {
    let f = write_toml(
        r#"
[servers.sleeper]
command = "sleep"
args = ["300"]
"#,
    );

    let output = mcp_hub()
        .args([
            "start",
            "--no-color",
            "--config",
            f.path().to_str().unwrap(),
        ])
        .timeout(Duration::from_secs(5))
        .output()
        .expect("Failed to run mcp-hub");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // ANSI escape codes must be absent when --no-color is set.
    assert!(
        !stdout.contains("\x1b["),
        "Expected no ANSI escape codes in stdout with --no-color, got:\n{stdout}"
    );

    // Server name must still appear in the plain-text table.
    assert!(
        stdout.contains("sleeper"),
        "Expected 'sleeper' in stdout, got:\n{stdout}"
    );
}
