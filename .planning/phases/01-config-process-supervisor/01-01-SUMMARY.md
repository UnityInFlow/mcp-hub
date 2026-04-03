# Plan 01-01 Summary: Project Scaffolding + TOML Config Parsing

**Executed:** 2026-04-02
**Status:** Complete
**Requirements addressed:** CFG-01, CFG-02

---

## What Was Built

### Task 1: Cargo project initialized
- `Cargo.toml` with all Phase 1 dependencies: tokio, clap (with env feature), serde, serde_json, toml, anyhow, thiserror, tracing, tracing-subscriber, nix, tokio-util, dirs, comfy-table, owo-colors, rand
- `src/main.rs` with `#[tokio::main]` entry point and module declarations
- `src/lib.rs` exposing public modules for integration tests

### Task 2: Shared domain types (`src/types.rs`)
- `ProcessState` enum with 6 variants: `Stopped`, `Starting`, `Running`, `Backoff { attempt, until }`, `Fatal`, `Stopping`
- `Display` impl: maps each variant to a human-readable string ("stopped", "backoff (N)", etc.)
- `BackoffConfig` struct with `Default` impl (1.0s base, 60.0s max, 0.3 jitter, 10 attempts, 60s stable window)
- `#[allow(dead_code)]` at module level — these types are consumed by later plans

### Task 3: CLI structure (`src/cli.rs`)
- `Cli` struct with global flags: `--no-color` (env: `NO_COLOR`), `-v/--verbose` (Count), `--config/-c`
- `Commands` enum: `Start`, `Stop`, `Restart(RestartArgs)`
- `RestartArgs` with positional `name: String`
- All types derive `Debug`; clap macros: `Parser`, `Subcommand`, `Args`

### Task 4: Config parsing (`src/config.rs`)
- `ServerConfig`: `command` (required), `args`, `env`, `env_file`, `transport` (default "stdio"), `cwd`, plus Phase 2 override fields
- `HubConfig`: `servers: HashMap<String, ServerConfig>` with `#[serde(default)]`
- `load_config()`: two-pass approach — first raw `toml::Value` for unknown field warnings, then typed deserialization
- `validate_config()`: collects ALL errors before returning (not fail-fast) — catches empty command and invalid transport
- `resolve_env()`: merges `env_file` into inline `env`, file values take precedence (D-04)
- `find_and_load_config()`: explicit path > global `~/.config/mcp-hub/mcp-hub.toml` > local `./mcp-hub.toml`; local overrides global by server name

### Task 5: Test fixtures and unit tests
- 6 fixture files covering valid config, missing command, bad transport, unknown fields, env override, and test env file
- 7 unit tests in `tests/config_test.rs` — all pass

---

## Verification Results

| Check | Result |
|-------|--------|
| `cargo build` | Passed |
| `cargo test --test config_test` | 7/7 passed |
| `cargo clippy -- -D warnings` | No issues |
| `cargo fmt -- --check` | No diff |

---

## Deviations from Plan

- `clap` features extended from `["derive"]` to `["derive", "env"]` — required for `env = "NO_COLOR"` attribute support in clap 4
- `tokio-util` features changed from `["sync"]` to `["codec", "io"]` — `sync` is not a valid feature in tokio-util 0.7
- Added `src/lib.rs` to expose modules for integration tests (not mentioned in plan but required by the test approach)
- Added `#![allow(dead_code)]` to `src/config.rs` and `src/types.rs` to pass `cargo clippy -- -D warnings` for public API not yet consumed by main.rs

---

## Commits

1. `feat(01-01-T1): initialize Cargo project and configure Phase 1 dependencies`
2. `feat(01-01-T2): define ProcessState enum and BackoffConfig struct with Default impl`
3. `feat(01-01-T3): define CLI structure with clap derive (Cli, Commands, RestartArgs)`
4. `feat(01-01-T4): implement TOML config parsing, validation, env_file merging, and config resolution`
5. `test(01-01-T5): add config test fixtures and 7 unit tests for config parsing`

---

## Next Plan

**Plan 02:** Process Supervisor — spawn child processes, SIGTERM/SIGKILL shutdown, exponential backoff auto-restart.
