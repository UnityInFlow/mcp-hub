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

/// Parse stdout as JSON, panicking with the raw text if it is not valid.
fn parse_stdout_json(stdout: &[u8]) -> serde_json::Value {
    let text = String::from_utf8(stdout.to_vec()).expect("stdout is valid UTF-8");
    serde_json::from_str::<serde_json::Value>(&text)
        .unwrap_or_else(|e| panic!("stdout must be pure JSON ({e}), got:\n{text}"))
}

/// `--format claude` produces a parseable JSON document on stdout containing
/// both servers under `mcpServers`. Exit code must be 0.
#[test]
fn claude_format_outputs_valid_json() {
    let fixture = write_fixture(two_servers_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mcpServers\""));

    let value = parse_stdout_json(&assert.get_output().stdout);
    let servers = value["mcpServers"]
        .as_object()
        .expect("mcpServers must be an object");
    assert!(servers.contains_key("filesystem"));
    assert!(servers.contains_key("github"));
    assert_eq!(servers["filesystem"]["command"], "npx");
    assert_eq!(servers["github"]["env"]["GITHUB_TOKEN"], "ghp_test123");
}

/// `--format cursor` produces a parseable JSON document on stdout; the
/// Cursor destination hint goes to stderr.
#[test]
fn cursor_format_outputs_valid_json() {
    let fixture = write_fixture(two_servers_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "cursor", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("~/.cursor/mcp.json"))
        .stdout(predicate::str::contains("//").not());

    let value = parse_stdout_json(&assert.get_output().stdout);
    let servers = value["mcpServers"]
        .as_object()
        .expect("mcpServers must be an object");
    assert!(servers.contains_key("filesystem"));
    assert!(servers.contains_key("github"));
}

/// A server with a non-empty env still yields pure JSON on stdout.
#[test]
fn env_config_stdout_parses_as_json() {
    let fixture = write_fixture(two_servers_toml());

    for format in ["claude", "cursor"] {
        let assert = mcp_hub()
            .args(["gen-config", "--format", format, "-c"])
            .arg(fixture.path())
            .assert()
            .success();
        let stdout = &assert.get_output().stdout;
        let value = parse_stdout_json(stdout);
        assert_eq!(
            value["mcpServers"]["github"]["env"]["GITHUB_TOKEN"],
            "ghp_test123"
        );
        assert!(
            stdout.ends_with(b"}\n"),
            "stdout must end with the JSON document and a newline"
        );
    }
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

    let assert = mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value["mcpServers"]
        .as_object()
        .expect("mcpServers must be an object")
        .is_empty());
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

/// HTTP transport server is reported in a `// WARNING:` line on stderr and
/// excluded from the mcpServers JSON on stdout. The stdio server is included.
#[test]
fn http_transport_excluded() {
    let fixture = write_fixture(http_transport_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("WARNING:"))
        .stderr(predicate::str::contains("remote-api"))
        .stdout(predicate::str::contains("WARNING").not());

    let value = parse_stdout_json(&assert.get_output().stdout);
    let servers = value["mcpServers"]
        .as_object()
        .expect("mcpServers must be an object");
    assert!(servers.contains_key("local-stdio"));
    assert!(
        !servers.contains_key("remote-api"),
        "http server must not appear as a key in mcpServers"
    );
}

/// The `// Generated by mcp-hub v` header goes to stderr; stdout stays pure JSON.
#[test]
fn version_in_header() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("// Generated by mcp-hub v"))
        .stdout(predicate::str::starts_with("{"))
        .stdout(predicate::str::contains("Generated by").not());
}

/// `--live` without a running daemon exits non-zero with an error message
/// mentioning "daemon" or "running". The MCP_HUB_SOCKET env var is set to a
/// non-existent path to guarantee no daemon is reachable.
#[test]
fn live_flag_without_daemon_errors() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "claude", "--live", "-c"])
        .arg(fixture.path())
        .env("MCP_HUB_SOCKET", "/tmp/mcp-hub-test-nonexistent.sock")
        .assert()
        .failure()
        .stderr(predicate::str::contains("daemon").or(predicate::str::contains("running")));
}

/// `--live --format cursor` also exits non-zero without a running daemon.
#[test]
fn live_flag_cursor_without_daemon_errors() {
    let fixture = write_fixture(two_servers_toml());

    mcp_hub()
        .args(["gen-config", "--format", "cursor", "--live", "-c"])
        .arg(fixture.path())
        .env("MCP_HUB_SOCKET", "/tmp/mcp-hub-test-nonexistent.sock")
        .assert()
        .failure()
        .stderr(predicate::str::contains("daemon").or(predicate::str::contains("running")));
}

// ─────────────────────────────────────────────────────────────────────────────
// ${VAR} expansion (issue #36)
// ─────────────────────────────────────────────────────────────────────────────

fn expansion_toml() -> &'static str {
    r#"
[servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "${MY_TOKEN}" }
"#
}

#[test]
fn expanded_secret_appears_in_output_with_stderr_warning() {
    let f = write_fixture(expansion_toml());
    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(f.path())
        .env("MY_TOKEN", "abc")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"GITHUB_PERSONAL_ACCESS_TOKEN\": \"abc\"",
        ))
        .stdout(predicate::str::contains("${MY_TOKEN}").not())
        .stderr(predicate::str::contains("MY_TOKEN"))
        .stderr(predicate::str::contains("secret"));
}

#[test]
fn expanded_non_secret_has_no_warning() {
    let f = write_fixture(
        r#"
[servers.s]
command = "node"
env = { LEVEL = "${MY_LEVEL}" }
"#,
    );
    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(f.path())
        .env("MY_LEVEL", "debug")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"LEVEL\": \"debug\""))
        .stderr(predicate::str::contains("secret").not());
}

#[test]
fn unset_variable_fails_gen_config() {
    let f = write_fixture(expansion_toml());
    mcp_hub()
        .args(["gen-config", "--format", "claude", "-c"])
        .arg(f.path())
        .env_remove("MY_TOKEN")
        .assert()
        .failure()
        .stderr(predicate::str::contains("MY_TOKEN"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Codex CLI (TOML) and Gemini CLI (JSON) formats (issue #6)
// ─────────────────────────────────────────────────────────────────────────────

fn parse_stdout_toml(stdout: &[u8]) -> toml::Value {
    let text = String::from_utf8(stdout.to_vec()).expect("stdout is valid UTF-8");
    toml::from_str::<toml::Value>(&text)
        .unwrap_or_else(|e| panic!("stdout must be pure TOML ({e}), got:\n{text}"))
}

/// `--format codex` prints a pure TOML document with `[mcp_servers.<name>]`
/// tables; the destination hint goes to stderr as a `#` comment.
#[test]
fn codex_format_outputs_valid_toml() {
    let fixture = write_fixture(two_servers_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "codex", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("~/.codex/config.toml"))
        .stderr(predicate::str::contains("# Generated by mcp-hub"))
        .stdout(predicate::str::contains("Generated by").not());

    let value = parse_stdout_toml(&assert.get_output().stdout);
    let servers = &value["mcp_servers"];
    assert_eq!(servers["filesystem"]["command"].as_str(), Some("npx"));
    assert_eq!(
        servers["filesystem"]["args"][2].as_str(),
        Some("/tmp"),
        "args must round-trip"
    );
    assert_eq!(
        servers["github"]["env"]["GITHUB_TOKEN"].as_str(),
        Some("ghp_test123")
    );
}

/// Server names that are not bare TOML keys are quoted so the document
/// still parses and the name round-trips.
#[test]
fn codex_format_quotes_non_bare_names() {
    let fixture = write_fixture(
        r#"
[servers."my.server"]
command = "node"

[servers."a b"]
command = "deno"
args = ["run"]
"#,
    );

    let assert = mcp_hub()
        .args(["gen-config", "--format", "codex", "-c"])
        .arg(fixture.path())
        .assert()
        .success();

    let value = parse_stdout_toml(&assert.get_output().stdout);
    assert_eq!(
        value["mcp_servers"]["my.server"]["command"].as_str(),
        Some("node")
    );
    assert_eq!(
        value["mcp_servers"]["a b"]["command"].as_str(),
        Some("deno")
    );
}

/// `cwd` is emitted for Codex and Gemini when set (both support it).
#[test]
fn codex_and_gemini_emit_cwd_when_set() {
    let fixture = write_fixture(
        r#"
[servers.app]
command = "node"
cwd = "/tmp"
"#,
    );

    let codex = mcp_hub()
        .args(["gen-config", "--format", "codex", "-c"])
        .arg(fixture.path())
        .assert()
        .success();
    let value = parse_stdout_toml(&codex.get_output().stdout);
    assert_eq!(value["mcp_servers"]["app"]["cwd"].as_str(), Some("/tmp"));

    let gemini = mcp_hub()
        .args(["gen-config", "--format", "gemini", "-c"])
        .arg(fixture.path())
        .assert()
        .success();
    let value = parse_stdout_json(&gemini.get_output().stdout);
    assert_eq!(value["mcpServers"]["app"]["cwd"], "/tmp");
}

/// `--format gemini` prints a pure JSON document with `mcpServers`.
#[test]
fn gemini_format_outputs_valid_json() {
    let fixture = write_fixture(two_servers_toml());

    let assert = mcp_hub()
        .args(["gen-config", "--format", "gemini", "-c"])
        .arg(fixture.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("~/.gemini/settings.json"))
        .stdout(predicate::str::contains("//").not());

    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["mcpServers"]["filesystem"]["command"], "npx");
    assert_eq!(
        value["mcpServers"]["github"]["env"]["GITHUB_TOKEN"],
        "ghp_test123"
    );
}

/// HTTP-transport servers are excluded from both formats with a warning on
/// stderr; the stdio server stays.
#[test]
fn codex_and_gemini_exclude_http_transport() {
    let fixture = write_fixture(http_transport_toml());

    for format in ["codex", "gemini"] {
        mcp_hub()
            .args(["gen-config", "--format", format, "-c"])
            .arg(fixture.path())
            .assert()
            .success()
            .stderr(predicate::str::contains("WARNING: 'remote-api' uses http"))
            .stdout(predicate::str::contains("remote-api").not())
            .stdout(predicate::str::contains("local-stdio"));
    }
}

/// `--live` without a daemon fails for both new formats.
#[test]
fn live_flag_codex_gemini_without_daemon_errors() {
    let fixture = write_fixture(two_servers_toml());

    for format in ["codex", "gemini"] {
        mcp_hub()
            .args(["gen-config", "--format", format, "--live", "-c"])
            .arg(fixture.path())
            .env("MCP_HUB_SOCKET", "/tmp/mcp-hub-test-nonexistent.sock")
            .assert()
            .failure()
            .stderr(predicate::str::contains("daemon").or(predicate::str::contains("running")));
    }
}

/// Zero servers: warning on stderr, valid empty document on stdout.
#[test]
fn codex_and_gemini_zero_servers_warn() {
    let fixture = write_fixture(empty_servers_toml());

    for format in ["codex", "gemini"] {
        let assert = mcp_hub()
            .args(["gen-config", "--format", format, "-c"])
            .arg(fixture.path())
            .assert()
            .success()
            .stderr(predicate::str::contains("No servers configured"));
        if format == "codex" {
            parse_stdout_toml(&assert.get_output().stdout);
        } else {
            parse_stdout_json(&assert.get_output().stdout);
        }
    }
}

/// `--help` lists every format.
#[test]
fn help_lists_formats() {
    mcp_hub()
        .args(["gen-config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex"))
        .stdout(predicate::str::contains("gemini"))
        .stdout(predicate::str::contains("cursor"));
}
