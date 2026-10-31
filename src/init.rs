use anyhow::Context as _;
use dialoguer::{Input, Select};
use std::io::Write as _;
use std::path::Path;

/// Escape a string for placement inside a TOML double-quoted basic string.
///
/// Replaces `\` with `\\` and `"` with `\"`. TOML basic strings forbid raw
/// control characters, so `\t`, `\n` and `\r` use their short escapes and every
/// other control character (U+0000-U+001F, U+007F) becomes `\uXXXX`.
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Split a line of arguments, shell-style but minimal.
///
/// - Arguments are separated by whitespace.
/// - A double-quoted section groups text, which may contain spaces and commas.
/// - Inside quotes, `\"` is a literal quote; any other backslash is literal
///   (so Windows paths like `"C:\Program Files\x"` work).
/// - `""` yields an explicit empty argument.
/// - An unterminated quote is an error.
pub fn parse_args(input: &str) -> anyhow::Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => in_quotes = false,
                '\\' if chars.peek() == Some(&'"') => {
                    chars.next();
                    current.push('"');
                }
                c => current.push(c),
            }
        } else if c == '"' {
            in_quotes = true;
            in_token = true;
        } else if c.is_whitespace() {
            if in_token {
                args.push(std::mem::take(&mut current));
                in_token = false;
            }
        } else {
            current.push(c);
            in_token = true;
        }
    }

    if in_quotes {
        anyhow::bail!("Unterminated double quote in arguments");
    }
    if in_token {
        args.push(current);
    }
    Ok(args)
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
/// For new (or blank) files the leading `\n` in `toml_block` is trimmed so the
/// file does not start with a blank line. For existing files the block is
/// appended after ensuring a trailing newline (the block's leading `\n`
/// provides the blank-line separator).
///
/// The write is atomic: the full new content is built in memory, written to
/// `<path>.tmp` in the same directory, synced, then renamed over `path`.
/// An existing file that is not valid TOML is never modified; an error is
/// returned instead.
pub fn write_server_entry_to(path: &Path, toml_block: &str) -> anyhow::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(e).with_context(|| format!("Failed to read {}", path.display()));
        }
    };

    let new_content = match existing {
        Some(existing) if !existing.trim().is_empty() => {
            toml::from_str::<toml::Value>(&existing).with_context(|| {
                format!(
                    "Existing {} is not valid TOML; refusing to modify it (fix or remove it first)",
                    path.display()
                )
            })?;
            let mut content = existing;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(toml_block);
            content
        }
        Some(_) | None => toml_block.trim_start_matches('\n').to_string(),
    };

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory for {}", path.display())
            })?;
        }
    }

    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "mcp-hub.toml".into());
    tmp_name.push(".tmp");
    let tmp_path = path.with_file_name(tmp_name);

    let result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("Failed to create {}", tmp_path.display()))?;
        file.write_all(new_content.as_bytes())
            .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", tmp_path.display()))?;
        drop(file);
        std::fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "Failed to rename {} over {}",
                tmp_path.display(),
                path.display()
            )
        })
    })();

    if result.is_err() {
        // Best-effort cleanup; the original error is what matters.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
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
        .with_prompt(
            "Arguments (space-separated; use \"double quotes\" to group, \\\" for a literal quote; Enter to skip)",
        )
        .default(String::new())
        .allow_empty(true)
        .validate_with(|input: &String| -> Result<(), String> {
            parse_args(input).map(|_| ()).map_err(|e| e.to_string())
        })
        .interact_text()
        .context("Failed to read arguments")?;

    let args: Vec<String> = parse_args(&args_raw).context("Invalid arguments")?;

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

    // -- toml_escape: control characters -------------------------------------

    #[test]
    fn toml_escape_control_chars() {
        assert_eq!(toml_escape("a\tb\nc\rd"), "a\\tb\\nc\\rd");
        assert_eq!(toml_escape("\u{1b}"), "\\u001B");
        assert_eq!(toml_escape("\u{7f}"), "\\u007F");
        assert_eq!(toml_escape("\u{0}"), "\\u0000");
    }

    // -- parse_args ------------------------------------------------------------

    #[test]
    fn parse_args_plain() {
        assert_eq!(
            parse_args("a b  c\t d").expect("ok"),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn parse_args_quoted_with_space() {
        assert_eq!(
            parse_args(r#"--name "hello world" x"#).expect("ok"),
            vec!["--name", "hello world", "x"]
        );
    }

    #[test]
    fn parse_args_quoted_with_comma() {
        assert_eq!(
            parse_args(r#""--filter=a,b" x"#).expect("ok"),
            vec!["--filter=a,b", "x"]
        );
    }

    #[test]
    fn parse_args_escaped_quote_inside_quotes() {
        assert_eq!(
            parse_args(r#""say \"hi\"" y"#).expect("ok"),
            vec![r#"say "hi""#, "y"]
        );
    }

    #[test]
    fn parse_args_empty_and_blank() {
        assert!(parse_args("").expect("ok").is_empty());
        assert!(parse_args("   \t ").expect("ok").is_empty());
    }

    #[test]
    fn parse_args_explicit_empty_string_arg() {
        assert_eq!(parse_args(r#"a "" b"#).expect("ok"), vec!["a", "", "b"]);
    }

    #[test]
    fn parse_args_unterminated_quote_errors() {
        assert!(parse_args(r#"a "b c"#).is_err());
    }

    // -- is_valid_server_name: Unicode rejection ------------------------------

    #[test]
    fn rejects_unicode_server_names() {
        assert!(!is_valid_server_name("cafe\u{0301}")); // e + combining accent
        assert!(!is_valid_server_name("\u{6570}\u{636e}")); // CJK: 数据
        assert!(!is_valid_server_name("serveur-francais-e\u{0301}"));
    }
}
