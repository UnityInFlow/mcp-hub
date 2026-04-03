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

/// Per-server configuration block.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
pub struct HubConfig {
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
    let config: HubConfig = toml::from_str(&content)
        .with_context(|| format!("Config schema error in {}", path.display()))?;

    validate_config(&config)?;
    Ok(config)
}

/// Validate all server entries in `config`, collecting all errors before failing.
pub fn validate_config(config: &HubConfig) -> anyhow::Result<()> {
    let mut errors: Vec<String> = Vec::new();

    for (name, server) in &config.servers {
        if server.command.is_empty() {
            errors.push(format!("Server '{}': 'command' must not be empty", name));
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
