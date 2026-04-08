use anyhow::Context as _;
use dialoguer::{Input, Select};
use std::io::Write as _;
use std::path::Path;

/// Escape a string for placement inside a TOML double-quoted basic string.
/// Replaces `\` with `\\` and `"` with `\"`.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Characters that are invalid in TOML bare keys and would break the generated config.
/// TOML bare keys only allow ASCII A-Za-z0-9, `-`, and `_`.
fn is_valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_'))
}

/// Return all server names found in `./mcp-hub.toml` in the current directory.
///
/// Returns an empty vec if the file does not exist or cannot be parsed.
/// Logs a warning if the file exists but fails to parse (per D-05).
pub fn existing_server_names() -> Vec<String> {
    existing_server_names_from(Path::new("./mcp-hub.toml"))
}

/// Return all server names found in the TOML file at `path`.
///
/// Returns an empty vec if the file does not exist or cannot be parsed.
/// Logs a warning if the file exists but fails to parse.
pub fn existing_server_names_from(path: &Path) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Could not read {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let value: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "Could not parse {} as TOML (ignored): {}",
                path.display(),
                e
            );
            return Vec::new();
        }
    };

    value
        .get("servers")
        .and_then(|v| v.as_table())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Build a hand-crafted TOML server block string.
///
/// - Omits `args` when the slice is empty (per D-10).
/// - Omits `transport` when it is `"stdio"` (the default, per D-10).
/// - Starts with a leading newline so appends are separated from existing content.
pub fn format_toml_block(name: &str, command: &str, args: &[String], transport: &str) -> String {
    let mut block = format!(
        "\n[servers.{name}]\ncommand = \"{}\"\n",
        toml_escape(command)
    );

    if !args.is_empty() {
        let quoted: Vec<String> = args
            .iter()
            .map(|a| format!("\"{}\"", toml_escape(a)))
            .collect();
        block.push_str(&format!("args = [{}]\n", quoted.join(", ")));
    }

    if transport != "stdio" {
        block.push_str(&format!("transport = \"{}\"\n", toml_escape(transport)));
    }

    block
}

/// Write `toml_block` to `./mcp-hub.toml` in the current directory.
///
/// Appends if the file exists, creates it if it does not.
pub fn write_server_entry(toml_block: &str) -> anyhow::Result<()> {
    write_server_entry_to(Path::new("./mcp-hub.toml"), toml_block)
}

/// Write `toml_block` to `path`, appending if the file exists.
///
/// For new files the leading `\n` in `toml_block` is trimmed so the file
/// does not start with a blank line. For existing files the content is
/// appended directly (the leading `\n` provides the blank-line separator).
pub fn write_server_entry_to(path: &Path, toml_block: &str) -> anyhow::Result<()> {
    if path.exists() {
        // Read first to check trailing newline, then open for append.
        let existing = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open {} for appending", path.display()))?;

        // Ensure the existing file ends with a newline before appending.
        if !existing.ends_with('\n') {
            file.write_all(b"\n")
                .with_context(|| format!("Failed to write newline to {}", path.display()))?;
        }

        file.write_all(toml_block.as_bytes())
            .with_context(|| format!("Failed to write to {}", path.display()))?;
    } else {
        // New file — trim the leading newline to avoid a blank first line.
        let content = toml_block.trim_start_matches('\n');
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create parent directory for {}", path.display())
                })?;
            }
        }
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }

    Ok(())
}

/// Run the interactive `mcp-hub init` wizard.
///
/// Prompts for: server name, command, args (optional), transport (Select).
/// Writes the new server entry to `./mcp-hub.toml` (creates or appends).
pub fn run_init_wizard() -> anyhow::Result<()> {
    let existing_names = existing_server_names();

    // ── Prompt 1: server name ──────────────────────────────────────────────
    let name: String = Input::new()
        .with_prompt("Server name")
        .validate_with(|input: &String| -> Result<(), String> {
            if input.is_empty() {
                return Err("Server name must not be empty".to_string());
            }
            if !is_valid_server_name(input) {
                return Err(
                    "Server name may only contain letters, digits, hyphens, and underscores"
                        .to_string(),
                );
            }
            if existing_names.contains(input) {
                let list = existing_names.join(", ");
                return Err(format!(
                    "Name '{input}' already exists. Existing servers: {list}"
                ));
            }
            Ok(())
        })
        .interact_text()
        .context("Failed to read server name")?;

    // ── Prompt 2: command ─────────────────────────────────────────────────
    let command: String = Input::new()
        .with_prompt("Command")
        .validate_with(|input: &String| -> Result<(), String> {
            if input.is_empty() {
                Err("Command must not be empty".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()
        .context("Failed to read command")?;

    // ── Prompt 3: args (optional) ─────────────────────────────────────────
    let args_raw: String = Input::new()
        .with_prompt("Arguments (comma-separated, Enter to skip)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()
        .context("Failed to read arguments")?;

    let args: Vec<String> = args_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // ── Prompt 4: transport ───────────────────────────────────────────────
    let transports = &["stdio", "http"];
    let transport_idx = Select::new()
        .with_prompt("Transport")
        .items(transports)
        .default(0)
        .interact()
        .context("Failed to read transport selection")?;

    let transport = transports[transport_idx];

    // ── Write to file ─────────────────────────────────────────────────────
    let toml_block = format_toml_block(&name, &command, &args, transport);
    write_server_entry(&toml_block)
        .with_context(|| format!("Failed to write server '{name}' to ./mcp-hub.toml"))?;

    println!("Added '{name}' to ./mcp-hub.toml");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_toml_block tests ────────────────────────────────────────────

    #[test]
    fn format_block_with_args_and_stdio_transport() {
        let block = format_toml_block(
            "github",
            "npx",
            &["@anthropic/mcp-github".to_string()],
            "stdio",
        );
        assert!(block.contains("[servers.github]"), "missing header");
        assert!(block.contains("command = \"npx\""), "missing command");
        assert!(
            block.contains("args = [\"@anthropic/mcp-github\"]"),
            "missing args"
        );
        // stdio is the default — must be omitted
        assert!(
            !block.contains("transport"),
            "transport must be omitted for stdio"
        );
    }

    #[test]
    fn format_block_with_http_transport() {
        let block = format_toml_block("api", "python", &[], "http");
        assert!(block.contains("[servers.api]"), "missing header");
        assert!(block.contains("command = \"python\""), "missing command");
        // no args — must be omitted
        assert!(!block.contains("args"), "args must be omitted when empty");
        assert!(block.contains("transport = \"http\""), "missing transport");
    }

    #[test]
    fn format_block_empty_args_omits_args_line() {
        let block = format_toml_block("test", "echo", &[], "stdio");
        assert!(
            !block.contains("args"),
            "args line must be absent when slice is empty"
        );
    }

    #[test]
    fn format_block_stdio_omits_transport_line() {
        let block = format_toml_block("test", "echo", &[], "stdio");
        assert!(
            !block.contains("transport"),
            "transport line must be absent for stdio default"
        );
    }

    #[test]
    fn format_block_multiple_args() {
        let args = vec!["server.py".to_string(), "--port=8080".to_string()];
        let block = format_toml_block("api", "python", &args, "http");
        assert!(
            block.contains("args = [\"server.py\", \"--port=8080\"]"),
            "args must be quoted and comma-separated"
        );
        assert!(block.contains("transport = \"http\""), "transport present");
    }

    #[test]
    fn format_block_starts_with_newline_for_append_separation() {
        let block = format_toml_block("s", "cmd", &[], "stdio");
        assert!(
            block.starts_with('\n'),
            "block must start with newline for append separation"
        );
    }

    // ── existing_server_names_from tests ──────────────────────────────────

    #[test]
    fn existing_names_from_missing_file_returns_empty() {
        let names = existing_server_names_from(Path::new("/tmp/nonexistent_mcp_hub_abc123.toml"));
        assert!(names.is_empty(), "must return empty vec for missing file");
    }

    #[test]
    fn existing_names_from_valid_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mcp-hub.toml");
        std::fs::write(
            &path,
            "[servers.github]\ncommand = \"npx\"\n\n[servers.filesystem]\ncommand = \"python\"\n",
        )
        .expect("write");

        let mut names = existing_server_names_from(&path);
        names.sort();
        assert_eq!(names, vec!["filesystem", "github"]);
    }

    #[test]
    fn existing_names_from_malformed_file_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mcp-hub.toml");
        std::fs::write(&path, "this is [[ not valid toml {{").expect("write");

        let names = existing_server_names_from(&path);
        assert!(names.is_empty(), "must return empty on parse error");
    }

    // ── is_valid_server_name tests ────────────────────────────────────────

    #[test]
    fn valid_server_names() {
        assert!(is_valid_server_name("github"));
        assert!(is_valid_server_name("my-server"));
        assert!(is_valid_server_name("server_1"));
        assert!(is_valid_server_name("MCP1"));
    }

    #[test]
    fn invalid_server_names() {
        assert!(!is_valid_server_name(""));
        assert!(!is_valid_server_name("has space"));
        assert!(!is_valid_server_name("has.dot"));
        assert!(!is_valid_server_name("[brackets]"));
        assert!(!is_valid_server_name("has\nnewline"));
        assert!(!is_valid_server_name("has\"quote"));
    }

    // -- toml_escape tests ---------------------------------------------------

    #[test]
    fn toml_escape_plain_string_unchanged() {
        assert_eq!(toml_escape("hello"), "hello");
    }

    #[test]
    fn toml_escape_backslash() {
        assert_eq!(toml_escape("C:\\tools\\mcp.exe"), "C:\\\\tools\\\\mcp.exe");
    }

    #[test]
    fn toml_escape_double_quote() {
        assert_eq!(toml_escape("node \"server.js\""), "node \\\"server.js\\\"");
    }

    #[test]
    fn toml_escape_both() {
        assert_eq!(toml_escape("back\\and\"quote"), "back\\\\and\\\"quote");
    }

    // -- is_valid_server_name: Unicode rejection ------------------------------

    #[test]
    fn rejects_unicode_server_names() {
        assert!(!is_valid_server_name("cafe\u{0301}")); // e + combining accent
        assert!(!is_valid_server_name("\u{6570}\u{636e}")); // CJK: 数据
        assert!(!is_valid_server_name("serveur-francais-e\u{0301}"));
    }
}
