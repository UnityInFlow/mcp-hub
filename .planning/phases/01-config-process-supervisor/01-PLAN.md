---
plan_id: "01-01"
title: "Project scaffolding + TOML config parsing"
phase: 1
wave: 1
depends_on: []
files_modified:
  - Cargo.toml
  - src/main.rs
  - src/cli.rs
  - src/config.rs
  - src/types.rs
  - tests/fixtures/valid.toml
  - tests/fixtures/invalid-missing-command.toml
  - tests/fixtures/invalid-bad-transport.toml
  - tests/fixtures/unknown-fields.toml
  - tests/fixtures/env-override.toml
  - tests/fixtures/test.env
  - tests/config_test.rs
requirements_addressed:
  - CFG-01
  - CFG-02
autonomous: true
---

# Plan 01: Project Scaffolding + TOML Config Parsing

<objective>
Create the Rust project skeleton with Cargo.toml, define all shared types (ProcessState, ServerConfig, HubConfig, BackoffConfig), implement TOML config loading with validation (clear errors for invalid entries, warnings for unknown fields), env_file merging, and config file resolution (local + global). All config parsing logic is unit-tested with fixture files.
</objective>

---

## Task 1: Initialize Cargo project and configure dependencies

<task id="01-01-T1">
<read_first>
- .planning/research/STACK.md (dependency versions and rationale)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (decisions D-01 through D-17)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 10: File Structure for Phase 1)
- CLAUDE.md (Rust constraints: edition 2021, no unwrap, clippy, fmt)
</read_first>

<action>
Run `cargo init --name mcp-hub` in the project root directory.

Replace the generated `Cargo.toml` with the following content:

```toml
[package]
name = "mcp-hub"
version = "0.0.1"
edition = "2021"
description = "PM2 for MCP servers — manage, monitor, and configure your MCP servers"
license = "MIT"
repository = "https://github.com/UnityInFlow/mcp-hub"
keywords = ["mcp", "process-manager", "ai", "developer-tools"]
categories = ["command-line-utilities", "development-tools"]

[dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
nix = { version = "0.29", features = ["signal", "process"] }
tokio-util = { version = "0.7", features = ["sync"] }
dirs = "5"
comfy-table = "7"
owo-colors = { version = "4", features = ["supports-colors"] }
rand = "0.9"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
tokio-test = "0.4"
```

Create `src/main.rs` with a minimal tokio entry point that parses CLI args and prints version:

```rust
mod cli;
mod config;
mod types;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Subcommand dispatch will be added in Plan 03
    println!("mcp-hub v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

Verify the project compiles with `cargo build`.
</action>

<acceptance_criteria>
- File `Cargo.toml` exists and contains `name = "mcp-hub"`
- File `Cargo.toml` contains `edition = "2021"`
- File `Cargo.toml` contains `tokio = { version = "1", features = ["full"] }`
- File `Cargo.toml` contains `clap = { version = "4", features = ["derive"] }`
- File `Cargo.toml` contains `toml = "0.8"`
- File `Cargo.toml` contains `nix = { version = "0.29", features = ["signal", "process"] }`
- File `Cargo.toml` contains `dirs = "5"`
- File `Cargo.toml` contains `rand = "0.9"`
- File `src/main.rs` exists and contains `#[tokio::main]`
- File `src/main.rs` contains `mod cli;` and `mod config;` and `mod types;`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 2: Define shared domain types

<task id="01-01-T2">
<read_first>
- src/main.rs (to see module declarations)
- .planning/research/ARCHITECTURE.md (ServerConfig, HubConfig, ProcessState definitions)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 5: state machine enum, BackoffConfig)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-03 through D-06, D-11 through D-14)
</read_first>

<action>
Create `src/types.rs` with the following types:

1. `ProcessState` enum with variants: `Stopped`, `Starting`, `Running`, `Backoff { attempt: u32, until: std::time::Instant }`, `Fatal`, `Stopping`. Derive `Debug, Clone, PartialEq, Eq`.

2. `BackoffConfig` struct with fields:
   - `base_delay_secs: f64` (default 1.0)
   - `max_delay_secs: f64` (default 60.0)
   - `jitter_factor: f64` (default 0.3)
   - `max_attempts: u32` (default 10)
   - `stable_window_secs: u64` (default 60)
   Implement `Default` for `BackoffConfig` with these values.

3. `ProcessState` display implementation: `Stopped` -> "stopped", `Starting` -> "starting", `Running` -> "running", `Backoff { attempt, .. }` -> "backoff (N)", `Fatal` -> "fatal", `Stopping` -> "stopping".

All types use `pub` visibility.
</action>

<acceptance_criteria>
- File `src/types.rs` exists
- `src/types.rs` contains `pub enum ProcessState`
- `ProcessState` has exactly 6 variants: `Stopped`, `Starting`, `Running`, `Backoff`, `Fatal`, `Stopping`
- `src/types.rs` contains `pub struct BackoffConfig`
- `BackoffConfig` has `base_delay_secs`, `max_delay_secs`, `jitter_factor`, `max_attempts`, `stable_window_secs` fields
- `BackoffConfig` `Default` impl has `base_delay_secs: 1.0`, `max_delay_secs: 60.0`, `jitter_factor: 0.3`, `max_attempts: 10`, `stable_window_secs: 60`
- No `unwrap()` in the file
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 3: Define CLI structure with clap derive

<task id="01-01-T3">
<read_first>
- src/main.rs (to see how Cli is imported)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 6: CLI Design with clap Derive — full code patterns)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-15 through D-17)
</read_first>

<action>
Create `src/cli.rs` with the following clap derive definitions:

1. `Cli` struct (top-level parser):
   - `#[command(name = "mcp-hub", version, about = "PM2 for MCP servers — manage, monitor, and configure your MCP servers")]`
   - `no_color: bool` — `#[arg(long, global = true, env = "NO_COLOR")]`
   - `verbose: u8` — `#[arg(short = 'v', action = clap::ArgAction::Count, global = true)]`
   - `config: Option<std::path::PathBuf>` — `#[arg(long, short = 'c', global = true, value_name = "PATH")]`
   - `command: Commands` — `#[command(subcommand)]`

2. `Commands` enum (subcommands):
   - `Start` — "Start all configured MCP servers"
   - `Stop` — "Stop all running servers"
   - `Restart(RestartArgs)` — "Restart a specific server by name"

3. `RestartArgs` struct:
   - `name: String` — positional argument, help "Name of the server to restart"

All types derive `Debug` and use `pub` visibility. `Cli` derives `Parser`, `Commands` derives `Subcommand`, `RestartArgs` derives `clap::Args`.
</action>

<acceptance_criteria>
- File `src/cli.rs` exists
- `src/cli.rs` contains `pub struct Cli` with `#[derive(Parser`
- `src/cli.rs` contains `pub enum Commands` with `#[derive(Subcommand`
- `Commands` has variants `Start`, `Stop`, `Restart`
- `Cli` has fields `no_color`, `verbose`, `config`, `command`
- `no_color` has attribute `env = "NO_COLOR"`
- `verbose` uses `clap::ArgAction::Count`
- `src/cli.rs` contains `pub struct RestartArgs` with `pub name: String`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 4: Implement TOML config parsing and validation

<task id="01-01-T4">
<read_first>
- src/types.rs (ProcessState, BackoffConfig — already defined)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 1: TOML Config Schema Design — full code patterns including ServerConfig, HubConfig, load_config, validate_config, resolve_env, warn_unknown_fields)
- .planning/phases/01-config-process-supervisor/01-CONTEXT.md (D-01 through D-06: config shape, field definitions, validation rules)
- .planning/research/PITFALLS.md (Pitfall #8: transport field must be explicit)
</read_first>

<action>
Create `src/config.rs` with the following components:

1. `ServerConfig` struct with serde `Deserialize` and `Serialize`:
   - `command: String` (required)
   - `args: Vec<String>` (default empty via `#[serde(default)]`)
   - `env: HashMap<String, String>` (default empty via `#[serde(default)]`)
   - `env_file: Option<String>` (optional path to .env file)
   - `transport: String` (default "stdio" via `#[serde(default = "default_transport")]`)
   - `cwd: Option<String>` (optional working directory)
   - `health_check_interval: Option<u64>` (Phase 2 override)
   - `max_retries: Option<u32>` (Phase 2 override)
   - `restart_delay: Option<u64>` (Phase 2 override)
   Derive `Debug, Clone, Serialize, Deserialize`.

2. `HubConfig` struct:
   - `servers: HashMap<String, ServerConfig>` with `#[serde(default)]`
   Derive `Debug, Deserialize, Serialize`.

3. `fn default_transport() -> String` returning `"stdio".to_string()`.

4. `pub fn load_config(path: &std::path::Path) -> anyhow::Result<HubConfig>`:
   - Read file with `std::fs::read_to_string`
   - First pass: parse as `toml::Value` to detect unknown fields, emit `tracing::warn!` for each unknown key under `[servers.<name>]` that is not in the known set (command, args, env, env_file, transport, cwd, health_check_interval, max_retries, restart_delay)
   - Second pass: parse as `HubConfig` via `toml::from_str`
   - Call `validate_config(&config)?`
   - Return `Ok(config)`
   - All errors use `anyhow::Context` with the file path in the message

5. `pub fn validate_config(config: &HubConfig) -> anyhow::Result<()>`:
   - Collect all errors into a `Vec<String>` (show all problems, not just the first)
   - Check: `server.command` must not be empty for each server
   - Check: `server.transport` must be "stdio" or "http"
   - If errors vec is non-empty, join with newlines and `anyhow::bail!`

6. `pub fn resolve_env(server: &ServerConfig) -> anyhow::Result<HashMap<String, String>>`:
   - Clone `server.env` as base
   - If `env_file` is Some, read the file, parse KEY=VALUE lines (skip blank lines and `#` comments), trim whitespace
   - env_file values override inline env (per D-04)
   - Return the merged HashMap

7. `pub fn find_and_load_config(explicit_path: Option<&std::path::Path>) -> anyhow::Result<HubConfig>`:
   - If explicit_path is provided, load only that file
   - Otherwise: check `dirs::config_dir().join("mcp-hub/mcp-hub.toml")` (global) and `std::env::current_dir().join("mcp-hub.toml")` (local)
   - If both exist: load both, merge — `local.servers` overrides `global.servers` by name (use `extend`)
   - If neither exists: `anyhow::bail!("No mcp-hub.toml found. Create one or run `mcp-hub init`.")`

No `unwrap()` anywhere — use `?`, `anyhow::Context`, or `anyhow::bail!`.
</action>

<acceptance_criteria>
- File `src/config.rs` exists
- `src/config.rs` contains `pub struct ServerConfig` with `#[derive(` including `Deserialize`
- `ServerConfig` has fields: `command`, `args`, `env`, `env_file`, `transport`, `cwd`
- `src/config.rs` contains `pub struct HubConfig` with `servers: HashMap<String, ServerConfig>`
- `src/config.rs` contains `pub fn load_config(path: &std::path::Path) -> anyhow::Result<HubConfig>`
- `src/config.rs` contains `pub fn validate_config`
- `src/config.rs` contains `pub fn resolve_env`
- `src/config.rs` contains `pub fn find_and_load_config`
- `src/config.rs` does NOT contain `unwrap()`
- `cargo build` exits 0
</acceptance_criteria>
</task>

---

## Task 5: Create test fixtures and config unit tests

<task id="01-01-T5">
<read_first>
- src/config.rs (current implementation — test against actual API)
- src/types.rs (types used in assertions)
- .planning/phases/01-config-process-supervisor/01-RESEARCH.md (Section 1: example TOML, Section 10: unit tests table)
- .planning/phases/01-config-process-supervisor/01-VALIDATION.md (task 01-01, 01-06: config tests)
</read_first>

<action>
Create the following test fixture files:

`tests/fixtures/valid.toml`:
```toml
[servers.mcp-github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "placeholder" }
transport = "stdio"

[servers.mcp-filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]
cwd = "/home/user"
max_retries = 5
```

`tests/fixtures/invalid-missing-command.toml`:
```toml
[servers.broken]
command = ""
args = ["--verbose"]
```

`tests/fixtures/invalid-bad-transport.toml`:
```toml
[servers.bad-transport]
command = "echo"
transport = "grpc"
```

`tests/fixtures/unknown-fields.toml`:
```toml
[servers.with-extras]
command = "echo"
future_feature = true
some_new_option = "hello"
```

`tests/fixtures/env-override.toml`:
```toml
[servers.env-test]
command = "echo"
env = { KEY1 = "inline-value", KEY2 = "keep-this" }
env_file = "tests/fixtures/test.env"
```

`tests/fixtures/test.env`:
```
# This is a comment
KEY1=file-value
KEY3=new-key

```

Create `tests/config_test.rs` with unit tests:

1. `test_parse_valid_config`: load `valid.toml`, assert 2 servers exist, assert `mcp-github` has command "npx", assert `mcp-filesystem` has `cwd` = Some("/home/user"), assert `mcp-filesystem` has `max_retries` = Some(5).

2. `test_parse_missing_command_errors`: load `invalid-missing-command.toml`, assert `load_config` returns Err, assert error message contains "broken" and "command".

3. `test_parse_bad_transport_errors`: load `invalid-bad-transport.toml`, assert `load_config` returns Err, assert error message contains "grpc" or "transport".

4. `test_parse_unknown_fields_succeeds`: load `unknown-fields.toml`, assert `load_config` returns Ok (unknown fields are warnings, not errors), assert server "with-extras" exists with command "echo".

5. `test_env_file_overrides_inline`: load `env-override.toml`, call `resolve_env` on the `env-test` server, assert KEY1 = "file-value" (overridden by file), KEY2 = "keep-this" (only in inline), KEY3 = "new-key" (only in file).

6. `test_empty_config_has_no_servers`: parse an empty TOML string, assert `servers` HashMap is empty (not an error).

7. `test_transport_defaults_to_stdio`: parse a TOML with a server that omits the `transport` field, assert `transport == "stdio"`.

All tests use `std::path::PathBuf::from("tests/fixtures/...")` for fixture paths.
</action>

<acceptance_criteria>
- File `tests/fixtures/valid.toml` exists and contains `[servers.mcp-github]` and `[servers.mcp-filesystem]`
- File `tests/fixtures/invalid-missing-command.toml` exists and contains `command = ""`
- File `tests/fixtures/invalid-bad-transport.toml` exists and contains `transport = "grpc"`
- File `tests/fixtures/unknown-fields.toml` exists and contains `future_feature`
- File `tests/fixtures/env-override.toml` exists and contains `env_file`
- File `tests/fixtures/test.env` exists and contains `KEY1=file-value`
- File `tests/config_test.rs` exists
- `tests/config_test.rs` contains `test_parse_valid_config`
- `tests/config_test.rs` contains `test_parse_missing_command_errors`
- `tests/config_test.rs` contains `test_parse_bad_transport_errors`
- `tests/config_test.rs` contains `test_parse_unknown_fields_succeeds`
- `tests/config_test.rs` contains `test_env_file_overrides_inline`
- `cargo test --test config_test` exits 0
</acceptance_criteria>
</task>

---

<verification>
## Verification

Run these commands after all tasks are complete:

```bash
cargo build                    # Must exit 0 — project compiles
cargo test --test config_test  # Must exit 0 — all config tests pass
cargo clippy -- -D warnings    # Must exit 0 — no clippy warnings
cargo fmt -- --check           # Must exit 0 — code is formatted
```

### must_haves
- [ ] `Cargo.toml` has all Phase 1 dependencies (tokio, clap, serde, toml, nix, dirs, rand, comfy-table, owo-colors, tracing, tokio-util)
- [ ] `src/types.rs` defines `ProcessState` enum with 6 variants and `BackoffConfig` struct with Default impl
- [ ] `src/cli.rs` defines `Cli` (with `--no-color`, `-v`, `--config`), `Commands` (Start, Stop, Restart), `RestartArgs`
- [ ] `src/config.rs` implements `load_config`, `validate_config`, `resolve_env`, `find_and_load_config`
- [ ] Config validation catches empty commands and invalid transport values
- [ ] Unknown TOML fields produce warnings, not errors (forward-compatible)
- [ ] `env_file` values override inline `env` values
- [ ] All 7 config tests pass
- [ ] No `unwrap()` in any `src/` file
- [ ] `cargo clippy -- -D warnings` exits 0
</verification>
