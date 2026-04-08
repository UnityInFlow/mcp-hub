/// Integration tests for the `mcp-hub init` wizard building blocks.
///
/// These tests exercise `format_toml_block`, `write_server_entry_to`, and
/// `existing_server_names_from` without requiring interactive TTY input.
/// The full interactive wizard is verified manually via `mcp-hub init`.
use mcp_hub::init::{existing_server_names_from, format_toml_block, write_server_entry_to};

// ── write_server_entry_to: new file ───────────────────────────────────────────

#[test]
fn test_format_and_write_new_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mcp-hub.toml");

    let block = format_toml_block(
        "github",
        "npx",
        &["@anthropic/mcp-github".to_string()],
        "stdio",
    );
    write_server_entry_to(&path, &block).expect("write_server_entry_to");

    assert!(path.exists(), "mcp-hub.toml must be created");

    let content = std::fs::read_to_string(&path).expect("read");
    assert!(content.contains("[servers.github]"), "missing header");
    assert!(content.contains("command = \"npx\""), "missing command");
    assert!(
        content.contains("args = [\"@anthropic/mcp-github\"]"),
        "missing args"
    );
    assert!(
        !content.contains("transport"),
        "transport omitted for stdio"
    );

    // Must parse as valid TOML.
    let parsed: toml::Value = toml::from_str(&content).expect("must be valid TOML");
    assert!(
        parsed
            .get("servers")
            .and_then(|s| s.get("github"))
            .is_some(),
        "servers.github must exist in parsed TOML"
    );
}

// ── write_server_entry_to: append to existing ────────────────────────────────

#[test]
fn test_append_to_existing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mcp-hub.toml");

    // Write existing server first.
    let existing_content = "[servers.existing]\ncommand = \"echo\"\n";
    std::fs::write(&path, existing_content).expect("write existing");

    // Append a second server.
    let block = format_toml_block("newserver", "python", &["server.py".to_string()], "stdio");
    write_server_entry_to(&path, &block).expect("append");

    let content = std::fs::read_to_string(&path).expect("read");

    // Both servers must be present.
    assert!(
        content.contains("[servers.existing]"),
        "existing server preserved"
    );
    assert!(
        content.contains("[servers.newserver]"),
        "new server appended"
    );
    assert!(
        content.contains("command = \"echo\""),
        "existing command preserved"
    );
    assert!(
        content.contains("command = \"python\""),
        "new command present"
    );

    // Must parse as valid TOML with 2 servers.
    let parsed: toml::Value = toml::from_str(&content).expect("must be valid TOML after append");
    let servers = parsed
        .get("servers")
        .and_then(|s| s.as_table())
        .expect("servers table");
    assert_eq!(servers.len(), 2, "must have exactly 2 servers");
    assert!(servers.contains_key("existing"), "existing key present");
    assert!(servers.contains_key("newserver"), "newserver key present");
}

// ── existing_server_names_from ────────────────────────────────────────────────

#[test]
fn test_existing_server_names_from_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mcp-hub.toml");
    std::fs::write(
        &path,
        "[servers.alpha]\ncommand = \"a\"\n\n[servers.beta]\ncommand = \"b\"\n",
    )
    .expect("write");

    let mut names = existing_server_names_from(&path);
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn test_existing_server_names_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nonexistent.toml");

    let names = existing_server_names_from(&path);
    assert!(names.is_empty(), "must return empty vec for missing file");
}

// ── format_toml_block corner cases ────────────────────────────────────────────

#[test]
fn test_format_block_no_args_stdio() {
    let block = format_toml_block("test", "echo", &[], "stdio");
    assert!(block.contains("[servers.test]"), "header present");
    assert!(block.contains("command = \"echo\""), "command present");
    assert!(!block.contains("args"), "args omitted when empty");
    assert!(!block.contains("transport"), "transport omitted for stdio");
}

#[test]
fn test_format_block_with_args_http() {
    let block = format_toml_block(
        "api",
        "python",
        &["server.py".to_string(), "--port=8080".to_string()],
        "http",
    );
    assert!(
        block.contains("args = [\"server.py\", \"--port=8080\"]"),
        "args formatted correctly"
    );
    assert!(block.contains("transport = \"http\""), "transport included");
}

#[test]
fn test_format_block_args_omitted_when_empty() {
    let block = format_toml_block("srv", "cmd", &[], "http");
    assert!(
        !block.contains("args"),
        "args must be omitted when empty vec"
    );
    assert!(block.contains("transport = \"http\""), "transport present");
}

#[test]
fn test_new_file_no_leading_blank_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mcp-hub.toml");

    let block = format_toml_block("srv", "cmd", &[], "stdio");
    write_server_entry_to(&path, &block).expect("write");

    let content = std::fs::read_to_string(&path).expect("read");
    assert!(
        !content.starts_with('\n'),
        "new file must not start with a blank line"
    );
}

// -- TOML escaping roundtrip tests (CR-01 regression) ----------------------

#[test]
fn test_format_block_escapes_backslash_and_quote() {
    // Command with backslash (Windows path)
    let command = r#"C:\tools\mcp.exe"#;
    let block = format_toml_block("win", command, &[], "stdio");
    let content = block.trim_start_matches('\n');
    let parsed: toml::Value =
        toml::from_str(content).expect("TOML with escaped backslash must parse");
    let parsed_cmd = parsed["servers"]["win"]["command"].as_str().unwrap();
    assert_eq!(parsed_cmd, command, "backslash must roundtrip");
}

#[test]
fn test_format_block_escapes_double_quote_in_command() {
    let command = r#"node "my server.js""#;
    let block = format_toml_block("quoted", command, &[], "stdio");
    let content = block.trim_start_matches('\n');
    let parsed: toml::Value = toml::from_str(content).expect("TOML with escaped quote must parse");
    let parsed_cmd = parsed["servers"]["quoted"]["command"].as_str().unwrap();
    assert_eq!(parsed_cmd, command, "double-quote must roundtrip");
}

#[test]
fn test_format_block_escapes_args_with_special_chars() {
    let args = vec![
        r#"--config="my config.json""#.to_string(),
        r#"C:\Users\me\file.txt"#.to_string(),
    ];
    let block = format_toml_block("special", "cmd", &args, "stdio");
    let content = block.trim_start_matches('\n');
    let parsed: toml::Value = toml::from_str(content).expect("TOML with escaped args must parse");
    let parsed_args: Vec<String> = parsed["servers"]["special"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(parsed_args, args, "args with special chars must roundtrip");
}

#[test]
fn test_format_block_windows_path_full_roundtrip() {
    let command = r#"C:\Program Files\mcp\server.exe"#;
    let args = vec![r#"--dir=C:\Users\me\data"#.to_string()];
    let block = format_toml_block("winpath", command, &args, "stdio");

    // Write to temp file and read back — full file roundtrip
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mcp-hub.toml");
    write_server_entry_to(&path, &block).expect("write");

    let content = std::fs::read_to_string(&path).expect("read");
    let parsed: toml::Value =
        toml::from_str(&content).expect("file with Windows paths must parse as valid TOML");
    let srv = &parsed["servers"]["winpath"];
    assert_eq!(srv["command"].as_str().unwrap(), command);
    assert_eq!(srv["args"][0].as_str().unwrap(), args[0]);
}
