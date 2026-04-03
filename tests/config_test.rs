use mcp_hub::config::{load_config, resolve_env};
use std::path::PathBuf;

#[test]
fn test_parse_valid_config() {
    let path = PathBuf::from("tests/fixtures/valid.toml");
    let config = load_config(&path).expect("valid.toml should parse without error");

    assert_eq!(config.servers.len(), 2, "expected 2 servers");

    let github = config
        .servers
        .get("mcp-github")
        .expect("mcp-github server should exist");
    assert_eq!(github.command, "npx");

    let fs = config
        .servers
        .get("mcp-filesystem")
        .expect("mcp-filesystem server should exist");
    assert_eq!(fs.cwd, Some("/home/user".to_string()));
    assert_eq!(fs.max_retries, Some(5));
}

#[test]
fn test_parse_missing_command_errors() {
    let path = PathBuf::from("tests/fixtures/invalid-missing-command.toml");
    let result = load_config(&path);
    assert!(result.is_err(), "empty command should produce an error");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("broken") && err.contains("command"),
        "error should mention the server name and the 'command' field, got: {err}"
    );
}

#[test]
fn test_parse_bad_transport_errors() {
    let path = PathBuf::from("tests/fixtures/invalid-bad-transport.toml");
    let result = load_config(&path);
    assert!(result.is_err(), "bad transport should produce an error");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("grpc") || err.contains("transport"),
        "error should mention the invalid transport value, got: {err}"
    );
}

#[test]
fn test_parse_unknown_fields_succeeds() {
    let path = PathBuf::from("tests/fixtures/unknown-fields.toml");
    let config = load_config(&path).expect("unknown fields should produce warnings, not errors");

    let server = config
        .servers
        .get("with-extras")
        .expect("with-extras server should exist");
    assert_eq!(server.command, "echo");
}

#[test]
fn test_env_file_overrides_inline() {
    let path = PathBuf::from("tests/fixtures/env-override.toml");
    let config = load_config(&path).expect("env-override.toml should parse");

    let server = config
        .servers
        .get("env-test")
        .expect("env-test server should exist");

    let resolved = resolve_env(server).expect("resolve_env should succeed");

    assert_eq!(
        resolved.get("KEY1").map(|s| s.as_str()),
        Some("file-value"),
        "KEY1 should be overridden by env_file"
    );
    assert_eq!(
        resolved.get("KEY2").map(|s| s.as_str()),
        Some("keep-this"),
        "KEY2 should be kept from inline env"
    );
    assert_eq!(
        resolved.get("KEY3").map(|s| s.as_str()),
        Some("new-key"),
        "KEY3 should come from env_file"
    );
}

#[test]
fn test_empty_config_has_no_servers() {
    let config: mcp_hub::config::HubConfig =
        toml::from_str("").expect("empty TOML should deserialize to empty config");
    assert!(
        config.servers.is_empty(),
        "empty config should have no servers"
    );
}

#[test]
fn test_transport_defaults_to_stdio() {
    let toml_str = r#"
[servers.default-transport]
command = "echo"
"#;
    let config: mcp_hub::config::HubConfig =
        toml::from_str(toml_str).expect("TOML should deserialize");
    let server = config
        .servers
        .get("default-transport")
        .expect("default-transport server should exist");
    assert_eq!(server.transport, "stdio");
}
