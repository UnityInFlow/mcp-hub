#![allow(dead_code)]

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Known field names for a `[servers.<name>]` block.
const KNOWN_SERVER_FIELDS: &[&str] = &[
    "command",
    "args",
    "env",
    "env_file",
    "transport",
    "cwd",
    "health_check_interval",
    "max_retries",
    "restart_delay",
];

fn default_transport() -> String {
    "stdio".to_string()
}

fn default_web_port() -> u16 {
    3456
}

/// Global hub configuration from the optional `[hub]` section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HubGlobalConfig {
    /// Port for the web UI (default 3456). Per D-05, WEB-01.
    #[serde(default = "default_web_port")]
    pub web_port: u16,
}

impl Default for HubGlobalConfig {
    fn default() -> Self {
        Self {
            web_port: default_web_port(),
        }
    }
}

/// Per-server configuration block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    /// The executable to run (required, must not be empty).
    pub command: String,

    /// Command-line arguments passed to the executable.
    #[serde(default)]
    pub args: Vec<String>,

    /// Inline environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Path to a `.env` file whose values override `env`.
    pub env_file: Option<String>,

    /// Transport protocol ("stdio" or "http"). Defaults to "stdio".
    #[serde(default = "default_transport")]
    pub transport: String,

    /// Working directory for the child process.
    pub cwd: Option<String>,

    /// Per-server health check interval override (seconds). Phase 2.
    pub health_check_interval: Option<u64>,

    /// Per-server max restart attempts override. Phase 2.
    pub max_retries: Option<u32>,

    /// Per-server base restart delay override (seconds). Phase 2.
    pub restart_delay: Option<u64>,
}

/// Top-level config structure loaded from `mcp-hub.toml`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HubConfig {
    /// Global hub settings from the optional `[hub]` section.
    #[serde(default)]
    pub hub: HubGlobalConfig,
    /// Map of server name → server configuration.
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}

/// Emit `tracing::warn!` for any unknown fields found under `[servers.<name>]`.
///
/// Unknown fields are not errors — they allow forward-compatible config files.
fn warn_unknown_fields(raw: &toml::Value) {
    let Some(servers_table) = raw.get("servers").and_then(|v| v.as_table()) else {
        return;
    };
    for (server_name, server_value) in servers_table {
        let Some(fields) = server_value.as_table() else {
            continue;
        };
        for key in fields.keys() {
            if !KNOWN_SERVER_FIELDS.contains(&key.as_str()) {
                tracing::warn!(
                    server = %server_name,
                    field = %key,
                    "Unknown config field (ignored — may be from a newer mcp-hub version)"
                );
            }
        }
    }
}

/// Load and validate a config file at `path`.
///
/// Unknown fields produce warnings; invalid TOML or schema violations are errors.
pub fn load_config(path: &std::path::Path) -> anyhow::Result<HubConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;

    // First pass: detect unknown fields (forward-compatibility warnings).
    let raw: toml::Value =
        toml::from_str(&content).with_context(|| format!("Invalid TOML in {}", path.display()))?;
    warn_unknown_fields(&raw);

    // Second pass: typed deserialization.
    let mut config: HubConfig = toml::from_str(&content)
        .with_context(|| format!("Config schema error in {}", path.display()))?;

    // Relative `env_file` paths are relative to the config file, not the process cwd.
    if let Some(config_dir) = path.parent() {
        for server in config.servers.values_mut() {
            if let Some(env_file) = &server.env_file {
                let env_path = std::path::Path::new(env_file);
                if env_path.is_relative() {
                    server.env_file =
                        Some(config_dir.join(env_path).to_string_lossy().into_owned());
                }
            }
        }
    }

    validate_config(&config)?;
    Ok(config)
}

/// Validate all server entries in `config`, collecting all errors before failing.
pub fn validate_config(config: &HubConfig) -> anyhow::Result<()> {
    let mut errors: Vec<String> = Vec::new();

    for (name, server) in &config.servers {
        if name.trim().is_empty() {
            errors.push("Server name must not be empty".to_string());
        }
        if server.command.trim().is_empty() {
            errors.push(format!("Server '{}': 'command' must not be empty", name));
        }
        if server.health_check_interval == Some(0) {
            errors.push(format!(
                "Server '{}': 'health_check_interval' must be at least 1 second",
                name
            ));
        }
        if let Some(cwd) = &server.cwd {
            if !std::path::Path::new(cwd).is_dir() {
                errors.push(format!(
                    "Server '{}': 'cwd' is not an existing directory: {}",
                    name, cwd
                ));
            }
        }
        if !matches!(server.transport.as_str(), "stdio" | "http") {
            errors.push(format!(
                "Server '{}': unknown transport '{}' (expected 'stdio' or 'http')",
                name, server.transport
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("\n"))
    }
}

/// Merge `env_file` values into the server's inline `env` map.
///
/// Values from `env_file` take precedence over inline `env` (per D-04).
/// Lines starting with `#` and blank lines are ignored.
/// Windows line endings (`\r\n`) are handled by `str::trim`.
pub fn resolve_env(server: &ServerConfig) -> anyhow::Result<HashMap<String, String>> {
    let mut resolved = server.env.clone();

    if let Some(path) = &server.env_file {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read env_file: {path}"))?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                // env_file overrides inline env (D-04).
                resolved.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    Ok(resolved)
}

/// Find and load config from the default locations, or from `explicit_path` if given.
///
/// Search order:
/// 1. `explicit_path` (if provided)
/// 2. `~/.config/mcp-hub/mcp-hub.toml` (global)
/// 3. `./mcp-hub.toml` (local)
///
/// If both global and local exist, local servers override global by name.
pub fn find_and_load_config(explicit_path: Option<&std::path::Path>) -> anyhow::Result<HubConfig> {
    if let Some(path) = explicit_path {
        return load_config(path);
    }

    let global_path = dirs::config_dir().map(|d| d.join("mcp-hub").join("mcp-hub.toml"));

    let local_path = std::env::current_dir()
        .context("Cannot determine current directory")?
        .join("mcp-hub.toml");

    let global = global_path
        .filter(|p| p.exists())
        .map(|p| load_config(&p))
        .transpose()?;

    let local = if local_path.exists() {
        Some(load_config(&local_path)?)
    } else {
        None
    };

    match (global, local) {
        (None, None) => anyhow::bail!("No mcp-hub.toml found. Create one or run `mcp-hub init`."),
        (Some(g), None) => Ok(g),
        (None, Some(l)) => Ok(l),
        (Some(mut g), Some(l)) => {
            // Local servers override global by name (D-02).
            g.servers.extend(l.servers);
            Ok(g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_config_web_port() {
        let toml_str = r#"
[hub]
web_port = 8080

[servers.test]
command = "echo"
"#;
        let config: HubConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hub.web_port, 8080);
    }

    #[test]
    fn hub_config_default_port() {
        let toml_str = r#"
[servers.test]
command = "echo"
"#;
        let config: HubConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hub.web_port, 3456);
    }

    fn parse(toml_str: &str) -> HubConfig {
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn validate_rejects_zero_health_check_interval() {
        let config = parse(
            r#"
[servers.zero]
command = "echo"
health_check_interval = 0
"#,
        );
        let err = validate_config(&config).unwrap_err().to_string();
        assert!(
            err.contains("zero") && err.contains("health_check_interval"),
            "error should name the server and field, got: {err}"
        );
    }

    #[test]
    fn validate_accepts_positive_health_check_interval() {
        let config = parse(
            r#"
[servers.ok]
command = "echo"
health_check_interval = 1
"#,
        );
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn validate_allows_zero_restart_delay() {
        let config = parse(
            r#"
[servers.ok]
command = "echo"
restart_delay = 0
"#,
        );
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn validate_rejects_whitespace_only_command() {
        let config = parse(
            r#"
[servers.blank]
command = "   "
"#,
        );
        let err = validate_config(&config).unwrap_err().to_string();
        assert!(
            err.contains("blank") && err.contains("command"),
            "error should name the server and field, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_server_name() {
        let config = parse(
            r#"
[servers.""]
command = "echo"
"#,
        );
        let err = validate_config(&config).unwrap_err().to_string();
        assert!(
            err.contains("name"),
            "error should explain the empty name, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_nonexistent_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let config = parse(&format!(
            "[servers.badcwd]\ncommand = \"echo\"\ncwd = {:?}\n",
            missing.display().to_string()
        ));
        let err = validate_config(&config).unwrap_err().to_string();
        assert!(
            err.contains("badcwd") && err.contains("does-not-exist"),
            "error should name the server and the path, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_cwd_that_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        let config = parse(&format!(
            "[servers.filecwd]\ncommand = \"echo\"\ncwd = {:?}\n",
            file.display().to_string()
        ));
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn validate_accepts_existing_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let config = parse(&format!(
            "[servers.goodcwd]\ncommand = \"echo\"\ncwd = {:?}\n",
            dir.path().display().to_string()
        ));
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn load_config_resolves_relative_env_file_against_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secrets.env"), "FROM_FILE=yes\n").unwrap();
        let cfg_path = dir.path().join("mcp-hub.toml");
        std::fs::write(
            &cfg_path,
            "[servers.s]\ncommand = \"echo\"\nenv_file = \"secrets.env\"\n",
        )
        .unwrap();

        let config = load_config(&cfg_path).unwrap();
        let server = config.servers.get("s").unwrap();
        let env = resolve_env(server).unwrap();
        assert_eq!(env.get("FROM_FILE").map(String::as_str), Some("yes"));
    }

    #[test]
    fn load_config_keeps_absolute_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join("abs.env");
        std::fs::write(&env_path, "K=v\n").unwrap();
        let cfg_dir = tempfile::tempdir().unwrap();
        let cfg_path = cfg_dir.path().join("mcp-hub.toml");
        std::fs::write(
            &cfg_path,
            format!(
                "[servers.s]\ncommand = \"echo\"\nenv_file = {:?}\n",
                env_path.display().to_string()
            ),
        )
        .unwrap();

        let config = load_config(&cfg_path).unwrap();
        let server = config.servers.get("s").unwrap();
        assert_eq!(
            server.env_file.as_deref(),
            Some(env_path.display().to_string().as_str())
        );
    }
}
