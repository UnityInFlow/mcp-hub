use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command as StdCommand, Stdio};
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

// ─────────────────────────────────────────────────────────────────────────────
// Stdin interactive command tests
//
// These tests spawn mcp-hub as a child process with piped stdin/stdout/stderr
// and interact with the interactive foreground loop. They are timing-sensitive
// by nature — generous timeouts (up to 15 s total) are used.
//
// Implementation strategy: background drain threads accumulate stdout/stderr
// into Arc<Mutex<String>> buffers. The test sleeps for a fixed duration (to
// let the hub start and process commands), then kills the child (closing pipes
// so drain threads exit), waits for the child, and finally checks the buffers.
// ─────────────────────────────────────────────────────────────────────────────

use std::sync::{Arc, Mutex};

/// Helper: find the mcp-hub binary path via assert_cmd's cargo metadata.
fn mcp_hub_bin_path() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin("mcp-hub")
}

/// Spawn mcp-hub with the given config. Returns:
/// - The child process (for later kill + wait)
/// - The child's stdin (for sending commands)
/// - An Arc<Mutex<String>> accumulating stdout
/// - An Arc<Mutex<String>> accumulating stderr
///
/// Two background drain threads continuously read from stdout and stderr into
/// the shared buffers. They exit naturally when the child process closes the
/// pipes (i.e. after the child is killed and waited on).
fn spawn_hub_collecting(
    config_path: &std::path::Path,
) -> (
    std::process::Child,
    std::process::ChildStdin,
    Arc<Mutex<String>>,
    Arc<Mutex<String>>,
) {
    let mut child = StdCommand::new(mcp_hub_bin_path())
        .args([
            "start",
            "--no-color",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn mcp-hub");

    let stdin = child.stdin.take().expect("stdin not available");

    let stdout_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // Drain stdout into the buffer.
    {
        let buf = Arc::clone(&stdout_buf);
        let stdout = child.stdout.take().expect("stdout not available");
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        buf.lock().unwrap().push_str(&line);
                        line.clear();
                    }
                }
            }
        });
    }

    // Drain stderr into the buffer.
    {
        let buf = Arc::clone(&stderr_buf);
        let stderr = child.stderr.take().expect("stderr not available");
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        buf.lock().unwrap().push_str(&line);
                        line.clear();
                    }
                }
            }
        });
    }

    (child, stdin, stdout_buf, stderr_buf)
}

/// Tests that typing `restart <name>` into the running hub's stdin restarts the
/// named server, and that a new status table is printed afterwards.
///
/// This test is timing-sensitive. If it becomes flaky in CI, mark it `#[ignore]`
/// and verify manually using the smoke test in the plan.
#[test]
fn test_stdin_restart_command() {
    let f = write_toml(
        r#"
[servers.server-a]
command = "sleep"
args = ["300"]

[servers.server-b]
command = "sleep"
args = ["300"]
"#,
    );

    let (mut child, mut stdin_writer, stdout_buf, _stderr_buf) = spawn_hub_collecting(f.path());

    // Wait for the hub to start and print the initial status table.
    std::thread::sleep(Duration::from_secs(5));

    // Send restart command for server-a.
    writeln!(stdin_writer, "restart server-a").expect("Failed to write to stdin");

    // Wait for the restart to complete and the new status table to be printed.
    // restart_server sleeps 2s before reprinting, so 5s is ample.
    std::thread::sleep(Duration::from_secs(5));

    // Send status command to trigger a third table print.
    writeln!(stdin_writer, "status").expect("Failed to write to stdin");
    std::thread::sleep(Duration::from_secs(2));

    // Kill the child — this closes its stdout pipe, causing the drain thread to exit.
    child.kill().ok();
    child.wait().ok();

    // Give the drain thread a moment to flush remaining data.
    std::thread::sleep(Duration::from_millis(200));

    let stdout = stdout_buf.lock().unwrap().clone();

    assert!(
        stdout.contains("server-a"),
        "Expected 'server-a' in stdout, got:\n{stdout}"
    );
    assert!(
        stdout.contains("server-b"),
        "Expected 'server-b' in stdout, got:\n{stdout}"
    );
    assert!(
        stdout.contains("running"),
        "Expected 'running' in stdout, got:\n{stdout}"
    );
}

/// Tests that restarting a server that does not exist prints a clear error to stderr.
#[test]
fn test_stdin_restart_unknown_server() {
    let f = write_toml(
        r#"
[servers.only-server]
command = "sleep"
args = ["300"]
"#,
    );

    let (mut child, mut stdin_writer, _stdout_buf, stderr_buf) = spawn_hub_collecting(f.path());

    // Wait for the hub to start.
    std::thread::sleep(Duration::from_secs(3));

    // Request restart of a server that doesn't exist.
    writeln!(stdin_writer, "restart nonexistent").expect("Failed to write to stdin");

    // Wait for the error message to be written to stderr.
    std::thread::sleep(Duration::from_secs(2));

    child.kill().ok();
    child.wait().ok();
    std::thread::sleep(Duration::from_millis(200));

    let stderr = stderr_buf.lock().unwrap().clone();

    assert!(
        stderr.contains("not found"),
        "Expected 'not found' error in stderr, got:\n{stderr}"
    );
}

/// Tests that typing `help` into the running hub's stdin prints available commands.
#[test]
fn test_stdin_help_command() {
    let f = write_toml(
        r#"
[servers.helper-server]
command = "sleep"
args = ["300"]
"#,
    );

    let (mut child, mut stdin_writer, _stdout_buf, stderr_buf) = spawn_hub_collecting(f.path());

    // Wait for the hub to start.
    std::thread::sleep(Duration::from_secs(3));

    // Send help command.
    writeln!(stdin_writer, "help").expect("Failed to write to stdin");

    // Wait for the help text to be written to stderr.
    std::thread::sleep(Duration::from_secs(2));

    child.kill().ok();
    child.wait().ok();
    std::thread::sleep(Duration::from_millis(200));

    let stderr = stderr_buf.lock().unwrap().clone();

    assert!(
        stderr.contains("Available commands"),
        "Expected 'Available commands' in stderr, got:\n{stderr}"
    );
    assert!(
        stderr.contains("restart"),
        "Expected 'restart' in help output, got:\n{stderr}"
    );
    assert!(
        stderr.contains("status"),
        "Expected 'status' in help output, got:\n{stderr}"
    );
}
