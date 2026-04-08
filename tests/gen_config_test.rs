/// CLI integration tests for the `gen-config` subcommand.
///
/// Tests use `assert_cmd` to invoke the `mcp-hub` binary and `tempfile` to
/// create self-contained TOML config fixtures — no permanent fixture files.
use std::io::Write as _;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::NamedTempFile;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn mcp_hub() -> Command {
    Command::cargo_bin("mcp-hub").expect("mcp-hub binary not found")
}

fn write_fixture(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("Failed to create tempfile");
    f.write_all(content.as_bytes())
        .expect("Failed to write tempfile");
    f.flush().expect("Failed to flush tempfile");
    f
}

/// Two-server TOML with filesystem and github servers.
fn two_servers_toml() -> &'static str {
    r#"
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_test123" }
"#
}

/// TOML with no servers defined.
fn empty_servers_toml() -> &'static str {
    r#"
# No servers defined
"#
}

/// TOML with one HTTP transport server and one stdio server.
fn http_transport_toml() -> &'static str {
    r#"
[servers.remote-api]
command = "http://localhost:8080"
transport = "http"

[servers.local-stdio]
command = "node"
args = ["server.js"]
"#
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

/// `--format claude` produces valid JSON on stdout with both server names and
/// the "mcpServers" key. Exit code must be 0.
#[test]
fn claude_format_outputs_valid_json() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mcpServers\""))
        .stdout(predicate::str::contains("\"filesystem\""))
        .stdout(predicate::str::contains("\"github\""))
        .stdout(predicate::str::contains("\"command\": \"npx\""))
        .stdout(predicate::str::contains("\"GITHUB_TOKEN\": \"ghp_test123\""));
}

/// `--format cursor` produces JSON on stdout with the Cursor-specific header.
#[test]
fn cursor_format_has_cursor_header() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "cursor", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("~/.cursor/mcp.json"));
}

/// Empty config prints a warning to stderr and an empty mcpServers block to
/// stdout. Exit code is 0 (zero-server is not an error per D-10).
#[test]
fn zero_servers_warning() {
    let fixture = write_fixture(empty_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No servers configured"))
        .stdout(predicate::str::contains("\"mcpServers\": {}"));
}

/// Unknown format value exits non-zero and error output mentions the valid
/// format options ("claude" and "cursor").
#[test]
fn unknown_format_error() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "yaml", "-c"])
        .arg(fixture.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("claude"))
        .stderr(predicate::str::contains("cursor"));
}

/// Internal config fields must NOT appear in the generated JSON output.
/// Only "command", "args", and "env" are client-facing.
#[test]
fn no_internal_fields_leak() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"transport\"").not())
        .stdout(predicate::str::contains("\"cwd\"").not())
        .stdout(predicate::str::contains("\"env_file\"").not())
        .stdout(predicate::str::contains("\"health_check_interval\"").not())
        .stdout(predicate::str::contains("\"max_retries\"").not());
}

/// Omitting `--format` causes a non-zero exit (clap required-argument error).
#[test]
fn missing_format_flag() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "-c"])
        .arg(fixture.path())
        .assert()
        .failure();
}

/// HTTP transport server appears in `// WARNING:` comment and is excluded from
/// the mcpServers JSON block. The stdio server is included normally.
#[test]
fn http_transport_excluded() {
    let fixture = write_fixture(http_transport_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("// WARNING:"))
        .stdout(predicate::str::contains("remote-api"))
        .stdout(predicate::str::contains("\"local-stdio\""));

    // Verify "remote-api" does NOT appear as a JSON key inside mcpServers.
    // It may appear in the WARNING comment line, but not as `"remote-api":`.
    let output = String::from_utf8(assert.get_output().stdout.clone())
        .expect("stdout is valid UTF-8");

    // Find the JSON block start (first `{`) and check inside it.
    let json_start = output.find('{').expect("output must contain JSON block");
    let json_part = &output[json_start..];
    assert!(
        !json_part.contains("\"remote-api\""),
        "http server must not appear as a JSON key in mcpServers, got:\n{json_part}"
    );
}

/// Output starts with `// Generated by mcp-hub v` version comment.
#[test]
fn version_in_header() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::starts_with("// Generated by mcp-hub v"));
}
