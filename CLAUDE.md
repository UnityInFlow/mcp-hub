# mcp-hub — Local MCP Server Process Manager

## Project Overview

**Tool 07** in the [UnityInFlow](https://github.com/UnityInFlow) ecosystem.

PM2-equivalent for MCP servers. Manages lifecycle (start/stop/restart/status), health monitoring with auto-restart, unified log streaming, and config generation for Claude Code and Cursor. Single Rust binary, zero dependencies, instant startup.

**Phase:** 2 | **Stack:** Rust | **Distribution:** pre-built binaries + Homebrew + `cargo install`

## Status

Planned — no strict blocking dependencies.

## Reference Documents

- `07-mcp-hub.md` — Feature spec, key features checklist, technical stack, TOML config format, implementation todos (Weeks 13-16)
- `claude-code-harness-engineering-guide-v2.md` — Harness engineering patterns and best practices

Read these before making architectural or scope decisions.

## Tooling

| Tool | Status | Usage |
|---|---|---|
| **GSD** | Installed (global) | `/gsd:new-project` to scaffold when ready. `/gsd:plan-phase` and `/gsd:execute-phase` for structured development. |
| **RTK** | Active (v0.34.2) | Automatic via hooks. Compresses cargo, git output. ~80% token savings. |
| **Superpowers** | Active (v5.0.5) | Auto-triggers brainstorming, TDD, planning, code review, debugging skills. |

## Constraints

### Rust (inherited from ecosystem CLAUDE.md)
- Rust stable, edition 2021
- `clap` for CLI argument parsing (derive feature)
- `serde` + `serde_json` for serialisation
- `tokio` for async runtime
- `anyhow` for error handling in binaries, `thiserror` for libraries
- Format: `cargo fmt` before every commit
- Lint: `cargo clippy -- -D warnings` must pass
- Distribution: pre-built binaries for macOS (arm64/x86_64), Linux (x86_64/aarch64), Windows
- No `unwrap()` in production code — use `?` or handle the error
- Pattern match exhaustively — no catch-all `_` unless truly needed

### General
- Test coverage >80% on core logic before release
- No secrets committed — all credentials via environment variables

## Acceptance Criteria — v0.0.1

- [ ] TOML config file: define all MCP servers with name, command, args, env
- [ ] Process lifecycle: start, stop, restart, status for all servers
- [ ] Health monitoring: heartbeat checks with configurable interval
- [ ] Auto-restart on crash with exponential backoff (1s -> 2s -> 4s -> max 60s)
- [ ] Unified log view: `mcp-hub logs --follow` streams all servers interleaved
- [ ] Web UI: lists all running servers and available tools from MCP introspection
- [ ] Claude Code config generator: outputs correct settings.json mcpServers block
- [ ] Cursor config generator: outputs Cursor MCP config snippet
- [ ] `mcp-hub init`: interactive wizard to add a new server
- [ ] Pre-built binaries for macOS, Linux, Windows
- [ ] Homebrew formula

## Development Workflow

When ready to build:

1. `/gsd:new-project` — describe mcp-hub, feed existing spec
2. `/gsd:discuss-phase 1` — lock in decisions for Weeks 13-14 (process manager core: TOML config, process spawning, health checks, auto-restart, log aggregation)
3. `/gsd:plan-phase 1` — atomic task plans with file paths
4. `/gsd:execute-phase 1` — parallel execution with fresh context windows
5. `/gsd:discuss-phase 2` — lock in decisions for Weeks 14-15 (Axum web UI, MCP introspection, config generators, release)
6. `/gsd:plan-phase 2` — atomic task plans
7. `/gsd:execute-phase 2` — build and ship

Superpowers skills (TDD, code review, debugging) activate automatically during execution.

## Key Dependencies (for reference, not installed yet)

- `clap` — CLI argument parsing (derive)
- `tokio` — async runtime + process management
- `toml` — TOML config parsing
- `axum` — web UI server
- `serde` + `serde_json` — serialisation

---

## CI / Runners

CI and releases run on GitHub-hosted runners: `ubuntu-latest` (Linux) and `macos-14` / `macos-latest` (macOS). Do not use self-hosted runners.

---

## Do Not

- Do not use `unwrap()` in production code
- Do not add a JavaScript framework for the web UI — server-rendered HTML only
- Do not commit secrets or API keys
- Do not skip writing tests
- Do not inline the reference docs into this file — read them by path

<!-- GSD:project-start source:PROJECT.md -->
## Project

**mcp-hub**

A local MCP server process manager — PM2 for MCP servers. Single Rust binary that manages the lifecycle of multiple MCP servers (start/stop/restart/status), monitors health with auto-restart on crash, streams unified logs, introspects server capabilities via MCP protocol, and generates config snippets for Claude Code and Cursor. Zero runtime dependencies, instant startup.

**Core Value:** Developers running 5+ MCP servers can manage them all from one place — one config file, one command, one log stream — instead of copy-pasting startup commands across terminals.

### Constraints

- **Stack**: Rust stable (edition 2021) + Tokio async runtime — zero runtime dependencies, instant startup
- **CLI**: clap (derive feature) for argument parsing
- **Config**: TOML via toml crate — familiar to Rust/systems community
- **Web UI**: Axum + server-rendered HTML — no JavaScript framework, no npm required
- **Serialization**: serde + serde_json for all data structures
- **Error handling**: anyhow for binary, thiserror for library code
- **Quality**: cargo clippy -D warnings, cargo fmt, no unwrap() in production
- **CI**: GitHub-hosted runners (ubuntu-latest, macos-14 / macos-latest)
- **Distribution**: Pre-built binaries + Homebrew tap + cargo install
<!-- GSD:project-end -->

<!-- GSD:stack-start source:research/STACK.md -->
## Technology Stack

## Core Runtime
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio` | 1.x (latest stable) | Async runtime — process spawning, I/O, timers, signal handling | High |
| `clap` | 4.x (derive) | CLI argument parsing | High |
| `serde` + `serde_json` | 1.x | Serialization for config, MCP JSON-RPC, web API responses | High |
| `toml` | 0.8.x | TOML config file parsing | High |
| `anyhow` | 1.x | Error handling in binary (main, CLI) | High |
| `thiserror` | 2.x | Typed errors in library code | High |
## Web UI
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `axum` | 0.8.x | HTTP server for web UI and API endpoints | High |
| `askama` | 0.12.x | Compile-time HTML templating (Jinja2-like) | Medium-High |
| `tower-http` | 0.6.x | Static file serving, CORS, compression middleware | High |
- `actix-web` — different async runtime (actix), mixing with Tokio process management is painful
- `warp` — less actively maintained, more implicit routing
- `tera` — runtime template parsing, no compile-time checking
- Any JS framework — project constraint: server-rendered HTML only
## Process Management
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio::process` | (part of tokio) | Async child process spawning, stdin/stdout/stderr pipes | High |
| `tokio::signal` | (part of tokio) | Signal handling (SIGTERM, SIGINT, SIGHUP) | High |
| `nix` | 0.29.x | Unix-specific process management (process groups, SIGKILL, waitpid) | Medium-High |
- `duct` — synchronous, doesn't integrate with Tokio
- `subprocess` — also synchronous
- Raw `libc` — `nix` is the safe wrapper
## Daemon Mode
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `daemonize` | 0.5.x | Fork-to-background on Unix (setsid, close fds, redirect stdio) | Medium |
| `tokio::net::UnixListener` | (part of tokio) | IPC socket for CLI-to-daemon communication | High |
## MCP Protocol / JSON-RPC
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `serde_json` | (already included) | JSON-RPC 2.0 message construction/parsing | High |
| No MCP SDK needed | — | MCP over stdio is simple JSON-RPC; a thin custom client is sufficient | High |
- `initialize` (handshake)
- `tools/list`, `resources/list`, `prompts/list` (introspection)
- `ping` (health check)
- `jsonrpc-core` — server-oriented, not client; adds complexity for no benefit
- Any MCP SDK crate — too immature, unnecessary for 4 RPC calls
## Logging & Observability
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tracing` | 0.1.x | Structured logging framework | High |
| `tracing-subscriber` | 0.3.x | Log formatting, filtering, output | High |
- `log` + `env_logger` — older, no span support, worse async story
- `slog` — less ecosystem support than tracing
## Testing
| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio::test` | (part of tokio) | Async test runtime | High |
| `assert_cmd` | 2.x | CLI integration testing (run binary, check stdout/exit code) | High |
| `predicates` | 3.x | Fluent assertions for assert_cmd | High |
| `tempfile` | 3.x | Temporary directories for test configs/logs | High |
| `portpicker` | 0.1.x | Find free ports for test web servers | Medium |
## Build & Distribution
| Tool | Purpose | Confidence |
|------|---------|------------|
| `cargo` | Build system | High |
| `cross` | Cross-compilation for Linux/Windows from macOS | High |
| GitHub Actions + self-hosted runners | CI/CD | High |
| `cargo-dist` or manual | Binary releases + Homebrew formula generation | Medium |
## Summary: Cargo.toml Dependencies
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

Conventions not yet established. Will populate as patterns emerge during development.
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

Architecture not yet mapped. Follow existing patterns found in the codebase.
<!-- GSD:architecture-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd:quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd:debug` for investigation and bug fixing
- `/gsd:execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd:profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
