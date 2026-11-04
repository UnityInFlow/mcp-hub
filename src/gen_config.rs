//! Config generation module for Claude Code, Cursor, Codex CLI and Gemini CLI.
//!
//! Provides pure rendering functions that convert a `HubConfig` into
//! ready-to-paste config documents (JSON, or TOML for Codex CLI) for the
//! target client's MCP server config.
//! All rendering is offline — no daemon connection required.

use anyhow::Context as _;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::config::{self, HubConfig};
use crate::control::DaemonResponse;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Per-server live state for `--live` mode.
///
/// Populated from a daemon Status response; summarised on stderr alongside the
/// generated config (never inside the JSON document).
pub struct ServerLiveInfo {
    pub name: String,
    pub state: String,
    pub tool_names: Vec<String>,
    /// Number of MCP resources exposed by this server.
    pub resource_count: usize,
    /// Number of MCP prompts exposed by this server.
    pub prompt_count: usize,
}

/// Result of rendering a config snippet.
///
/// `json` is the config document (with trailing newline) destined for stdout
/// so it can be redirected straight into the client's config file. It is JSON
/// for every format except Codex CLI, where it is TOML (the field name is kept
/// for API stability). `notes` are human-readable comment lines (`//` for
/// JSON formats, `#` for TOML: version header, destination hint, warnings,
/// `--live` summary) destined for stderr.
pub struct RenderedConfig {
    pub json: String,
    pub notes: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Render a Claude Code `mcpServers` JSON block from `config`.
///
/// `json` is pure JSON. `notes` carries a `//` header with the mcp-hub version
/// and an ISO 8601 timestamp, a hint to paste the block into `~/.claude.json`
/// or `.mcp.json`, and any warnings.
///
/// If `live_info` is `Some`, `notes` also gets one summary line per server
/// (running state, tool/resource/prompt counts).
pub fn render_claude_config(
    config: &HubConfig,
    live_info: Option<&[ServerLiveInfo]>,
) -> anyhow::Result<RenderedConfig> {
    let destination_hint = "Paste into mcpServers in ~/.claude.json or .mcp.json";
    render_json_config(config, live_info, destination_hint, false)
}

/// Render a Cursor `mcpServers` JSON block from `config`.
///
/// Identical to [`render_claude_config`] except the destination hint
/// references `~/.cursor/mcp.json`.
pub fn render_cursor_config(
    config: &HubConfig,
    live_info: Option<&[ServerLiveInfo]>,
) -> anyhow::Result<RenderedConfig> {
    let destination_hint = "Paste into ~/.cursor/mcp.json or .cursor/mcp.json";
    render_json_config(config, live_info, destination_hint, false)
}

/// Render a Gemini CLI `mcpServers` JSON block from `config`.
///
/// Same shape as the Claude format; additionally emits `cwd` when a server
/// sets one (supported by Gemini CLI's stdio transport). Destination is the
/// user-scope `~/.gemini/settings.json` or project-scope `.gemini/settings.json`.
pub fn render_gemini_config(
    config: &HubConfig,
    live_info: Option<&[ServerLiveInfo]>,
) -> anyhow::Result<RenderedConfig> {
    let destination_hint =
        "Paste into mcpServers in ~/.gemini/settings.json or .gemini/settings.json";
    render_json_config(config, live_info, destination_hint, true)
}

/// Render a Codex CLI `[mcp_servers.<name>]` TOML document from `config`.
///
/// The document is built with the `toml` serializer so names that are not
/// bare keys (`my.server`, `a b`) are quoted correctly. `env` is omitted when
/// empty and `cwd` when unset. Notes use `#` comments.
pub fn render_codex_config(
    config: &HubConfig,
    live_info: Option<&[ServerLiveInfo]>,
) -> anyhow::Result<RenderedConfig> {
    let mut notes = header_notes(
        "#",
        "Append to ~/.codex/config.toml (or .codex/config.toml in a trusted project)",
    );
    notes.extend(build_warnings(config, "#"));

    let mut servers = toml::Table::new();
    for entry in collect_entries(config)? {
        let mut table = toml::Table::new();
        table.insert("command".to_string(), toml::Value::String(entry.command));
        table.insert(
            "args".to_string(),
            toml::Value::Array(entry.args.into_iter().map(toml::Value::String).collect()),
        );
        if !entry.env.is_empty() {
            let env: toml::Table = entry
                .env
                .into_iter()
                .map(|(k, v)| (k, toml::Value::String(v)))
                .collect();
            table.insert("env".to_string(), toml::Value::Table(env));
        }
        if let Some(cwd) = entry.cwd {
            table.insert("cwd".to_string(), toml::Value::String(cwd));
        }
        servers.insert(entry.name, toml::Value::Table(table));
    }

    let mut root = toml::Table::new();
    root.insert("mcp_servers".to_string(), toml::Value::Table(servers));
    let doc = toml::to_string(&root).context("Failed to serialize Codex CLI TOML")?;

    if let Some(info) = live_info {
        notes.extend(live_summary_lines(info, "#"));
    }

    Ok(RenderedConfig { json: doc, notes })
}

/// Parse a daemon `Status` response into a `Vec<ServerLiveInfo>`.
///
/// Returns `Err` if the response indicates failure (`ok == false`) or has an
/// unexpected structure. All JSON field access uses safe accessor methods —
/// no panics on malformed input (T-05-07 mitigation).
pub fn parse_live_info(response: &DaemonResponse) -> anyhow::Result<Vec<ServerLiveInfo>> {
    anyhow::ensure!(
        response.ok,
        "Daemon status query failed: {:?}",
        response.error
    );

    let data = response
        .data
        .as_ref()
        .context("Empty status response from daemon")?;

    let servers = data
        .as_array()
        .context("Expected array from status response")?;

    servers
        .iter()
        .map(|s| {
            Ok(ServerLiveInfo {
                name: s["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .context("Server entry missing 'name' field in status response")?
                    .to_string(),
                state: s["state"].as_str().unwrap_or("unknown").to_string(),
                tool_names: s["tool_names"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                resource_count: s["resources"].as_u64().unwrap_or(0) as usize,
                prompt_count: s["prompts"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// One stdio server ready to be rendered, with env already resolved.
struct ServerEntry {
    name: String,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<String>,
}

/// Collect the non-http servers in alphabetical order with resolved env.
///
/// HTTP transport servers are excluded (see [`build_warnings`]).
fn collect_entries(config: &HubConfig) -> anyhow::Result<Vec<ServerEntry>> {
    let mut names: Vec<&String> = config.servers.keys().collect();
    names.sort();

    let mut entries = Vec::new();
    for name in names {
        let server = &config.servers[name];
        if server.transport == "http" {
            continue;
        }
        let resolved_env = config::resolve_env(server)
            .with_context(|| format!("Failed to resolve env for server '{name}'"))?;
        entries.push(ServerEntry {
            name: name.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            env: resolved_env.into_iter().collect(),
            cwd: server.cwd.clone(),
        });
    }
    Ok(entries)
}

/// Version/timestamp header plus destination hint, as comment lines.
fn header_notes(prefix: &str, destination_hint: &str) -> Vec<String> {
    let version = env!("CARGO_PKG_VERSION");
    let timestamp = iso_timestamp();
    vec![
        format!("{prefix} Generated by mcp-hub v{version} at {timestamp}"),
        format!("{prefix} {destination_hint}"),
    ]
}

/// Shared rendering logic for the `mcpServers` JSON formats (Claude, Cursor,
/// Gemini). `with_cwd` adds a `cwd` key for servers that set one.
fn render_json_config(
    config: &HubConfig,
    live_info: Option<&[ServerLiveInfo]>,
    destination_hint: &str,
    with_cwd: bool,
) -> anyhow::Result<RenderedConfig> {
    let mut notes = header_notes("//", destination_hint);
    notes.extend(build_warnings(config, "//"));

    let mut mcp_servers: serde_json::Map<String, Value> = serde_json::Map::new();
    for entry in collect_entries(config)? {
        let mut value = json!({
            "command": entry.command,
            "args": entry.args,
            "env": entry.env,
        });
        if let (true, Some(cwd), Some(obj)) = (with_cwd, entry.cwd, value.as_object_mut()) {
            obj.insert("cwd".to_string(), Value::String(cwd));
        }
        mcp_servers.insert(entry.name, value);
    }

    let output_json = json!({ "mcpServers": Value::Object(mcp_servers) });
    let json_str = serde_json::to_string_pretty(&output_json)?;

    if let Some(info) = live_info {
        notes.extend(live_summary_lines(info, "//"));
    }

    Ok(RenderedConfig {
        json: format!("{json_str}\n"),
        notes,
    })
}

/// Return the current UTC time in RFC 3339 format with second precision.
///
/// Example: `2026-04-08T14:30:00Z`
fn iso_timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Collect `// WARNING:` lines for servers that use the `http` transport.
///
/// These lines are prepended to the output before the JSON block so the user
/// knows which servers were intentionally excluded.
fn build_warnings(config: &HubConfig, prefix: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut names: Vec<&String> = config.servers.keys().collect();
    names.sort();
    for name in names {
        let server = &config.servers[name];
        if server.transport == "http" {
            warnings.push(format!(
                "{prefix} WARNING: '{name}' uses http transport -- manual URL config required"
            ));
        }
    }
    warnings
}

/// Build one `//` summary line per server for `--live` mode.
///
/// Example: `// filesystem: running, tools=[read, write], resources=0, prompts=0`
/// Servers that are not running are flagged with a `WARNING:` prefix.
fn live_summary_lines(live_info: &[ServerLiveInfo], comment: &str) -> Vec<String> {
    live_info
        .iter()
        .map(|info| {
            let prefix = if info.state == "running" {
                ""
            } else {
                "WARNING: "
            };
            format!(
                "{comment} {prefix}{}: {}, tools=[{}], resources={}, prompts={}",
                info.name,
                info.state,
                info.tool_names.join(", "),
                info.resource_count,
                info.prompt_count
            )
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::{HubConfig, HubGlobalConfig, ServerConfig};

    fn make_server(command: &str, args: Vec<&str>) -> ServerConfig {
        ServerConfig {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: HashMap::new(),
            env_file: None,
            transport: "stdio".to_string(),
            cwd: None,
            health_check_interval: None,
            max_retries: None,
            restart_delay: None,
        }
    }

    fn make_config(servers: Vec<(&str, ServerConfig)>) -> HubConfig {
        HubConfig {
            hub: HubGlobalConfig::default(),
            servers: servers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_claude_format_two_servers() {
        let config = make_config(vec![
            ("alpha", make_server("npx", vec!["server-a"])),
            ("beta", make_server("node", vec!["server-b"])),
        ]);
        let output = render_claude_config(&config, None).unwrap().json;
        assert!(
            output.contains("\"mcpServers\""),
            "must contain mcpServers key"
        );
        assert!(output.contains("\"alpha\""), "must contain alpha server");
        assert!(output.contains("\"beta\""), "must contain beta server");
        assert!(
            output.contains("\"command\": \"npx\""),
            "must contain npx command"
        );
        assert!(
            output.contains("\"command\": \"node\""),
            "must contain node command"
        );
    }

    #[test]
    fn test_cursor_format_header() {
        let config = make_config(vec![
            ("alpha", make_server("npx", vec!["server-a"])),
            ("beta", make_server("node", vec!["server-b"])),
        ]);
        let rendered = render_cursor_config(&config, None).unwrap();
        let notes = rendered.notes.join("\n");
        assert!(
            notes.contains("cursor"),
            "cursor header must mention cursor"
        );
        assert!(
            notes.contains("~/.cursor/mcp.json"),
            "cursor header must mention ~/.cursor/mcp.json"
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&rendered.json).is_ok(),
            "cursor json must parse, got: {}",
            rendered.json
        );
    }

    #[test]
    fn test_version_comment() {
        let config = make_config(vec![("s", make_server("cmd", vec![]))]);
        let rendered = render_claude_config(&config, None).unwrap();
        assert!(
            rendered.notes[0].starts_with("// Generated by mcp-hub v"),
            "notes must start with version comment, got: {:?}",
            rendered.notes
        );
        assert!(
            !rendered.json.contains("//"),
            "json must not contain comments, got: {}",
            rendered.json
        );
    }

    #[test]
    fn test_timestamp_format() {
        let config = make_config(vec![("s", make_server("cmd", vec![]))]);
        let rendered = render_claude_config(&config, None).unwrap();
        // First note should contain an ISO 8601 timestamp with T and Z.
        let first_line = rendered.notes.first().unwrap();
        assert!(
            first_line.contains('T'),
            "timestamp must contain T separator"
        );
        assert!(first_line.ends_with('Z'), "timestamp must end with Z (UTC)");
    }

    #[test]
    fn test_zero_servers() {
        let config = make_config(vec![]);
        let output = render_claude_config(&config, None).unwrap().json;
        // The output must contain an empty mcpServers object.
        assert!(
            output.contains("\"mcpServers\": {}"),
            "zero servers must produce empty mcpServers object, got: {output}"
        );
    }

    #[test]
    fn test_alphabetical_ordering() {
        let config = make_config(vec![
            ("zulu", make_server("cmd-z", vec![])),
            ("alpha", make_server("cmd-a", vec![])),
        ]);
        let output = render_claude_config(&config, None).unwrap().json;
        let alpha_pos = output.find("\"alpha\"").expect("alpha must be present");
        let zulu_pos = output.find("\"zulu\"").expect("zulu must be present");
        assert!(
            alpha_pos < zulu_pos,
            "alpha must appear before zulu (alphabetical order)"
        );
    }

    #[test]
    fn test_http_transport_warning() {
        let mut http_server = make_server("http://localhost:8080", vec![]);
        http_server.transport = "http".to_string();

        let config = make_config(vec![("remote", http_server)]);
        let rendered = render_claude_config(&config, None).unwrap();
        let notes = rendered.notes.join("\n");

        assert!(
            notes.contains("// WARNING:"),
            "http transport must produce a WARNING note"
        );
        // The server name should appear in the warning but NOT in the JSON.
        assert!(
            notes.contains("remote"),
            "server name must appear in warning"
        );
        assert!(
            !rendered.json.contains("remote"),
            "http server must be excluded from mcpServers JSON block"
        );
    }

    #[test]
    fn test_env_passthrough() {
        let mut server = make_server("npx", vec![]);
        server
            .env
            .insert("API_KEY".to_string(), "secret123".to_string());

        let config = make_config(vec![("myserver", server)]);
        let output = render_claude_config(&config, None).unwrap().json;

        assert!(
            output.contains("\"API_KEY\": \"secret123\""),
            "env vars must appear in output"
        );
    }

    // ── Plan 02 tests ─────────────────────────────────────────────────────────

    use crate::control::DaemonResponse;
    use serde_json::json;

    #[test]
    fn test_parse_live_info_success() {
        let response = DaemonResponse::success(json!([{
            "name": "fs",
            "state": "running",
            "tool_names": ["read", "write"],
            "tools": 2,
            "resources": 0,
            "prompts": 0,
            "resource_names": [],
            "prompt_names": []
        }]));
        let result = parse_live_info(&response).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "fs");
        assert_eq!(result[0].state, "running");
        assert_eq!(result[0].tool_names, vec!["read", "write"]);
    }

    #[test]
    fn test_parse_live_info_failure() {
        let response = DaemonResponse::err("test error".to_string());
        let result = parse_live_info(&response);
        assert!(
            result.is_err(),
            "parse_live_info must return Err for failed response"
        );
    }

    #[test]
    fn test_parse_live_info_empty_array() {
        let response = DaemonResponse::success(json!([]));
        let result = parse_live_info(&response).unwrap();
        assert!(result.is_empty(), "empty array must produce empty Vec");
    }

    fn live_info(name: &str, state: &str, tools: &[&str]) -> Vec<ServerLiveInfo> {
        vec![ServerLiveInfo {
            name: name.to_string(),
            state: state.to_string(),
            tool_names: tools.iter().map(|s| s.to_string()).collect(),
            resource_count: 1,
            prompt_count: 2,
        }]
    }

    #[test]
    fn test_live_summary_running_with_tools() {
        let config = make_config(vec![("filesystem", make_server("npx", vec![]))]);
        let live = live_info("filesystem", "running", &["read", "write"]);
        let rendered = render_claude_config(&config, Some(&live)).unwrap();
        assert!(
            rendered.notes.iter().any(|n| n
                == "// filesystem: running, tools=[read, write], resources=1, prompts=2"),
            "running server must produce a summary note, got: {:?}",
            rendered.notes
        );
        assert!(
            !rendered.json.contains("//"),
            "live annotations must not leak into json, got: {}",
            rendered.json
        );
        assert!(serde_json::from_str::<serde_json::Value>(&rendered.json).is_ok());
    }

    #[test]
    fn test_live_summary_stopped_server() {
        let config = make_config(vec![("filesystem", make_server("npx", vec![]))]);
        let live = live_info("filesystem", "stopped", &[]);
        let rendered = render_claude_config(&config, Some(&live)).unwrap();
        assert!(
            rendered
                .notes
                .iter()
                .any(|n| n.starts_with("// WARNING: filesystem: stopped")),
            "stopped server must produce WARNING note, got: {:?}",
            rendered.notes
        );
    }

    #[test]
    fn test_live_summary_stopped_with_cached_tools() {
        let config = make_config(vec![("filesystem", make_server("npx", vec![]))]);
        let live = live_info("filesystem", "stopped", &["read"]);
        let rendered = render_claude_config(&config, Some(&live)).unwrap();
        assert!(
            rendered
                .notes
                .iter()
                .any(|n| n.starts_with("// WARNING: filesystem: stopped")
                    && n.contains("tools=[read]")),
            "stopped server with cached tools must report both, got: {:?}",
            rendered.notes
        );
    }

    #[test]
    fn test_live_server_named_env_keeps_json_valid() {
        let mut other = make_server("node", vec![]);
        other.env.insert("K".to_string(), "v".to_string());
        let config = make_config(vec![("env", make_server("npx", vec![])), ("other", other)]);
        let live = live_info("env", "running", &["t"]);
        let rendered = render_claude_config(&config, Some(&live)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered.json).unwrap();
        assert!(value["mcpServers"]["env"].is_object());
        assert_eq!(value["mcpServers"]["other"]["env"]["K"], "v");
    }

    // ── Codex CLI / Gemini CLI ────────────────────────────────────────────────

    #[test]
    fn test_codex_toml_round_trip_with_quoted_names() {
        let mut server = make_server("npx", vec!["-y", "pkg"]);
        server.env.insert("K".to_string(), "v\"q".to_string());
        let config = make_config(vec![
            ("my.server", server),
            ("a b", make_server("node", vec![])),
        ]);
        let rendered = render_codex_config(&config, None).unwrap();
        let value: toml::Value = toml::from_str(&rendered.json).unwrap();
        assert_eq!(
            value["mcp_servers"]["my.server"]["command"].as_str(),
            Some("npx")
        );
        assert_eq!(
            value["mcp_servers"]["my.server"]["args"][1].as_str(),
            Some("pkg")
        );
        assert_eq!(
            value["mcp_servers"]["my.server"]["env"]["K"].as_str(),
            Some("v\"q")
        );
        assert!(value["mcp_servers"]["a b"].get("env").is_none());
        assert!(rendered.json.contains("[mcp_servers.\"my.server\"]"));
    }

    #[test]
    fn test_codex_notes_use_hash_comments() {
        let mut http = make_server("http://x", vec![]);
        http.transport = "http".to_string();
        let config = make_config(vec![("remote", http), ("s", make_server("cmd", vec![]))]);
        let live = live_info("s", "running", &["t"]);
        let rendered = render_codex_config(&config, Some(&live)).unwrap();
        assert!(
            rendered.notes.iter().all(|n| n.starts_with("# ")),
            "{:?}",
            rendered.notes
        );
        assert!(rendered
            .notes
            .iter()
            .any(|n| n.contains("~/.codex/config.toml")));
        assert!(rendered
            .notes
            .iter()
            .any(|n| n.contains("WARNING: 'remote'")));
        assert!(rendered
            .notes
            .iter()
            .any(|n| n == "# s: running, tools=[t], resources=1, prompts=2"));
        assert!(!rendered.json.contains("remote"));
        assert!(!rendered.json.contains('#'));
    }

    #[test]
    fn test_codex_zero_servers_parses() {
        let rendered = render_codex_config(&make_config(vec![]), None).unwrap();
        assert!(
            toml::from_str::<toml::Value>(&rendered.json).is_ok(),
            "{}",
            rendered.json
        );
    }

    #[test]
    fn test_gemini_includes_cwd_but_claude_does_not() {
        let mut server = make_server("node", vec![]);
        server.cwd = Some("/work".to_string());
        let config = make_config(vec![("s", server)]);
        let gemini: serde_json::Value =
            serde_json::from_str(&render_gemini_config(&config, None).unwrap().json).unwrap();
        assert_eq!(gemini["mcpServers"]["s"]["cwd"], "/work");
        let claude: serde_json::Value =
            serde_json::from_str(&render_claude_config(&config, None).unwrap().json).unwrap();
        assert!(claude["mcpServers"]["s"].get("cwd").is_none());
    }

    #[test]
    fn test_gemini_hint() {
        let rendered = render_gemini_config(&make_config(vec![]), None).unwrap();
        assert!(rendered
            .notes
            .iter()
            .any(|n| n.contains("~/.gemini/settings.json")));
    }
}
