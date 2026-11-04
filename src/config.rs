use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

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

/// `transport` value for servers reached over HTTP instead of stdio.
pub const TRANSPORT_HTTP: &str = "http";

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

impl ServerConfig {
    /// True for `transport = "http"`: the process is supervised, but it does not
    /// speak JSON-RPC on stdio, so stdio health checks and introspection are skipped.
    pub fn is_http(&self) -> bool {
        self.transport == TRANSPORT_HTTP
    }
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

/// Error returned by [`expand_str`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    /// `${NAME}` referenced a variable that is not set and has no `:-default`.
    #[error(
        "environment variable '{name}' is not set (use ${{{name}:-default}} to provide a fallback)"
    )]
    Unset { name: String },
    /// A `${` was never closed by a matching `}`.
    #[error("unterminated '${{' (missing closing '}}'; write '$${{' for a literal '${{')")]
    Unterminated,
    /// The text after `${` is not a valid variable name (`[A-Za-z_][A-Za-z0-9_]*`).
    #[error("invalid variable name '{name}' in '${{...}}' (expected [A-Za-z_][A-Za-z0-9_]*)")]
    BadName { name: String },
}

/// True if `name` is a valid environment variable name: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        Some(_) | None => false,
    }
}

/// True if `name` looks like it holds a secret (contains `TOKEN`, `SECRET`,
/// `KEY` or `PASSWORD`, case-insensitive).
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["TOKEN", "SECRET", "KEY", "PASSWORD"]
        .iter()
        .any(|needle| upper.contains(needle))
}

/// Expand `${NAME}` and `${NAME:-default}` references in `input`.
///
/// Pure: variables are resolved only through `lookup`, which returns `None`
/// for an unset variable.
///
/// Rules:
/// - `${NAME}` becomes the value of `NAME`; if unset, [`ExpandError::Unset`].
///   A variable that is set to the empty string expands to the empty string.
/// - `${NAME:-default}` becomes the value of `NAME` if it is set *and*
///   non-empty, otherwise `default` (which may be empty). The default is
///   literal text up to the first `}`; it is not itself expanded.
/// - `$${` is an escape and yields a literal `${` (the rest is left as-is, so
///   `$${X}` becomes `${X}`).
/// - A lone `$`, or `$NAME` without braces, is kept literally.
/// - A `${` with no closing `}` is [`ExpandError::Unterminated`]; a name that
///   is not `[A-Za-z_][A-Za-z0-9_]*` is [`ExpandError::BadName`].
pub fn expand_str(
    input: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, ExpandError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];

        if let Some(escaped) = after.strip_prefix("${") {
            // `$${` -> literal `${`.
            out.push_str("${");
            rest = escaped;
        } else if let Some(body) = after.strip_prefix('{') {
            let end = body.find('}').ok_or(ExpandError::Unterminated)?;
            let inner = &body[..end];
            let (name, default) = match inner.split_once(":-") {
                Some((name, default)) => (name, Some(default)),
                None => (inner, None),
            };
            if !is_valid_var_name(name) {
                return Err(ExpandError::BadName {
                    name: name.to_string(),
                });
            }
            match (lookup(name), default) {
                (Some(value), Some(default)) => {
                    out.push_str(if value.is_empty() { default } else { &value })
                }
                (Some(value), None) => out.push_str(&value),
                (None, Some(default)) => out.push_str(default),
                (None, None) => {
                    return Err(ExpandError::Unset {
                        name: name.to_string(),
                    })
                }
            }
            rest = &body[end + 1..];
        } else {
            // Lone `$` or `$NAME`: literal.
            out.push('$');
            rest = after;
        }
    }

    out.push_str(rest);
    Ok(out)
}

/// Expand `${VAR}` references in every server's `env` values and `args`.
///
/// `command`, `cwd` and `env_file` are left untouched, and values read later
/// from `env_file` are never expanded (they are literal).
///
/// All errors are collected and reported together, each naming the server,
/// the field (`env.KEY` or `args[N]`) and the cause. On success, returns the
/// sorted, de-duplicated names of secret-looking variables
/// (see [`is_secret_name`]) that were actually read from the environment.
pub fn expand_config(
    config: &mut HubConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<String>> {
    let secrets: RefCell<BTreeSet<String>> = RefCell::new(BTreeSet::new());
    let tracking_lookup = |name: &str| {
        let value = lookup(name);
        if value.is_some() && is_secret_name(name) {
            secrets.borrow_mut().insert(name.to_string());
        }
        value
    };

    let mut errors: Vec<String> = Vec::new();
    let mut server_names: Vec<String> = config.servers.keys().cloned().collect();
    server_names.sort();

    for name in &server_names {
        let Some(server) = config.servers.get_mut(name) else {
            continue;
        };

        for (i, arg) in server.args.iter_mut().enumerate() {
            match expand_str(arg, tracking_lookup) {
                Ok(expanded) => *arg = expanded,
                Err(err) => errors.push(format!("Server '{name}': field 'args[{i}]': {err}")),
            }
        }

        let mut keys: Vec<String> = server.env.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let Some(value) = server.env.get_mut(&key) else {
                continue;
            };
            match expand_str(value, tracking_lookup) {
                Ok(expanded) => *value = expanded,
                Err(err) => errors.push(format!("Server '{name}': field 'env.{key}': {err}")),
            }
        }
    }

    if errors.is_empty() {
        Ok(secrets.into_inner().into_iter().collect())
    } else {
        anyhow::bail!(errors.join("\n"))
    }
}

/// Load and validate a config file at `path`.
///
/// Unknown fields produce warnings; invalid TOML, schema violations, or
/// unresolvable `${VAR}` references are errors. See [`load_config_report`].
// Used by the library API and tests; the binary goes through
// `load_config_report` (see #29 for the bin/lib double compilation).
pub fn load_config(path: &std::path::Path) -> anyhow::Result<HubConfig> {
    load_config_report(path).map(|(config, _secrets)| config)
}

/// Like [`load_config`], but also returns the names of secret-looking
/// environment variables (`TOKEN|SECRET|KEY|PASSWORD`) whose values were
/// expanded into the config, so callers that print the config (`gen-config`)
/// can warn.
///
/// `${VAR}` expansion against the hub's own environment happens here, after
/// parsing and before validation. Because it runs at load time, both the
/// supervisor (spawning) and `gen-config` see the expanded values.
pub fn load_config_report(path: &std::path::Path) -> anyhow::Result<(HubConfig, Vec<String>)> {
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

    // Expand `${VAR}` in env values and args against the hub's environment.
    let secret_vars = expand_config(&mut config, |name| std::env::var(name).ok())
        .map_err(|err| anyhow::anyhow!("Cannot expand variables in {}:\n{err}", path.display()))?;

    validate_config(&config)?;
    Ok((config, secret_vars))
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
    find_and_load_config_report(explicit_path).map(|(config, _secrets)| config)
}

/// Like [`find_and_load_config`], but also returns the secret-looking
/// variable names expanded into the config (see [`load_config_report`]).
pub fn find_and_load_config_report(
    explicit_path: Option<&std::path::Path>,
) -> anyhow::Result<(HubConfig, Vec<String>)> {
    if let Some(path) = explicit_path {
        return load_config_report(path);
    }

    let global_path = dirs::config_dir().map(|d| d.join("mcp-hub").join("mcp-hub.toml"));

    let local_path = std::env::current_dir()
        .context("Cannot determine current directory")?
        .join("mcp-hub.toml");

    let global = global_path
        .filter(|p| p.exists())
        .map(|p| load_config_report(&p))
        .transpose()?;

    let local = if local_path.exists() {
        Some(load_config_report(&local_path)?)
    } else {
        None
    };

    match (global, local) {
        (None, None) => anyhow::bail!("No mcp-hub.toml found. Create one or run `mcp-hub init`."),
        (Some(g), None) => Ok(g),
        (None, Some(l)) => Ok(l),
        (Some((mut g, mut g_secrets)), Some((l, l_secrets))) => {
            // Local servers override global by name (D-02).
            g.servers.extend(l.servers);
            g_secrets.extend(l_secrets);
            g_secrets.sort();
            g_secrets.dedup();
            Ok((g, g_secrets))
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

    fn lookup_from<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn expand_set_variable() {
        let out = expand_str("${FOO}", lookup_from(&[("FOO", "bar")])).unwrap();
        assert_eq!(out, "bar");
    }

    #[test]
    fn expand_unset_with_default_uses_default() {
        let out = expand_str("${FOO:-fallback}", lookup_from(&[])).unwrap();
        assert_eq!(out, "fallback");
    }

    #[test]
    fn expand_unset_with_empty_default_is_empty() {
        let out = expand_str("a${FOO:-}b", lookup_from(&[])).unwrap();
        assert_eq!(out, "ab");
    }

    #[test]
    fn expand_unset_without_default_errors() {
        let err = expand_str("${FOO}", lookup_from(&[])).unwrap_err();
        assert_eq!(
            err,
            ExpandError::Unset {
                name: "FOO".to_string()
            }
        );
    }

    #[test]
    fn expand_empty_set_with_default_uses_default() {
        let out = expand_str("${FOO:-fallback}", lookup_from(&[("FOO", "")])).unwrap();
        assert_eq!(out, "fallback");
    }

    #[test]
    fn expand_empty_set_without_default_is_empty() {
        let out = expand_str("x${FOO}y", lookup_from(&[("FOO", "")])).unwrap();
        assert_eq!(out, "xy");
    }

    #[test]
    fn expand_set_ignores_default() {
        let out = expand_str("${FOO:-fallback}", lookup_from(&[("FOO", "real")])).unwrap();
        assert_eq!(out, "real");
    }

    #[test]
    fn expand_escape_yields_literal() {
        let out = expand_str("$${X}", lookup_from(&[("X", "nope")])).unwrap();
        assert_eq!(out, "${X}");
    }

    #[test]
    fn expand_lone_dollar_is_literal() {
        let out = expand_str("cost $5 $", lookup_from(&[])).unwrap();
        assert_eq!(out, "cost $5 $");
    }

    #[test]
    fn expand_bare_name_is_literal() {
        let out = expand_str("$HOME and $$HOME", lookup_from(&[("HOME", "/h")])).unwrap();
        assert_eq!(out, "$HOME and $$HOME");
    }

    #[test]
    fn expand_unterminated_errors() {
        for input in ["${FOO", "${", "x${FOO:-bar"] {
            assert_eq!(
                expand_str(input, lookup_from(&[("FOO", "v")])).unwrap_err(),
                ExpandError::Unterminated,
                "input: {input}"
            );
        }
    }

    #[test]
    fn expand_bad_name_errors() {
        for input in ["${}", "${1X}", "${A-B}", "${A B}", "${:-x}"] {
            assert!(
                matches!(
                    expand_str(input, lookup_from(&[])),
                    Err(ExpandError::BadName { .. })
                ),
                "input: {input}"
            );
        }
    }

    #[test]
    fn expand_multiple_refs_and_adjacent_text() {
        let out = expand_str(
            "pre-${A}-mid-${B:-d}${C}-post",
            lookup_from(&[("A", "1"), ("C", "3")]),
        )
        .unwrap();
        assert_eq!(out, "pre-1-mid-d3-post");
    }

    #[test]
    fn expand_non_ascii_text_preserved() {
        let out = expand_str("héllo ${A} wörld", lookup_from(&[("A", "é")])).unwrap();
        assert_eq!(out, "héllo é wörld");
    }

    #[test]
    fn expand_config_names_server_field_and_var() {
        let mut config: HubConfig = toml::from_str(
            r#"
[servers.gh]
command = "echo"
args = ["--x=${MISSING_ARG}"]
env = { TOKEN = "${MISSING_ENV}" }
"#,
        )
        .unwrap();
        let err = expand_config(&mut config, lookup_from(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("gh"), "{err}");
        assert!(
            err.contains("args[0]") && err.contains("MISSING_ARG"),
            "{err}"
        );
        assert!(
            err.contains("env.TOKEN") && err.contains("MISSING_ENV"),
            "{err}"
        );
    }

    #[test]
    fn expand_config_leaves_command_cwd_and_env_file_alone() {
        let mut config: HubConfig = toml::from_str(
            r#"
[servers.s]
command = "${CMD}"
cwd = "${DIR}"
env_file = "${ENVF}"
"#,
        )
        .unwrap();
        expand_config(&mut config, lookup_from(&[])).unwrap();
        let s = &config.servers["s"];
        assert_eq!(s.command, "${CMD}");
        assert_eq!(s.cwd.as_deref(), Some("${DIR}"));
        assert_eq!(s.env_file.as_deref(), Some("${ENVF}"));
    }

    #[test]
    fn expand_config_reports_secret_vars_only_when_set() {
        let mut config: HubConfig = toml::from_str(
            r#"
[servers.s]
command = "echo"
env = { A = "${GH_TOKEN}", B = "${LEVEL}", C = "${UNSET_KEY:-x}" }
"#,
        )
        .unwrap();
        let secrets = expand_config(
            &mut config,
            lookup_from(&[("GH_TOKEN", "t"), ("LEVEL", "1")]),
        )
        .unwrap();
        assert_eq!(secrets, vec!["GH_TOKEN".to_string()]);
    }

    #[test]
    fn is_secret_name_matches_case_insensitively() {
        assert!(is_secret_name("GITHUB_TOKEN"));
        assert!(is_secret_name("my_Secret"));
        assert!(is_secret_name("API_KEY"));
        assert!(is_secret_name("dbPassword"));
        assert!(!is_secret_name("HOME"));
    }
}
