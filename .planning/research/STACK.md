# STACK.md — mcp-hub

> Research dimension: Standard 2025 Rust stack for a local MCP server process manager with web UI, health monitoring, and MCP protocol introspection.

---

## Core Runtime

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio` | 1.x (latest stable) | Async runtime — process spawning, I/O, timers, signal handling | High |
| `clap` | 4.x (derive) | CLI argument parsing | High |
| `serde` + `serde_json` | 1.x | Serialization for config, MCP JSON-RPC, web API responses | High |
| `toml` | 0.8.x | TOML config file parsing | High |
| `anyhow` | 1.x | Error handling in binary (main, CLI) | High |
| `thiserror` | 2.x | Typed errors in library code | High |

**Rationale:** This is the standard Rust systems tooling stack. Tokio is the only serious async runtime for process management (needs `tokio::process::Command`, signal handling, timers). clap derive is the standard for CLIs. toml crate is canonical.

---

## Web UI

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `axum` | 0.8.x | HTTP server for web UI and API endpoints | High |
| `askama` | 0.12.x | Compile-time HTML templating (Jinja2-like) | Medium-High |
| `tower-http` | 0.6.x | Static file serving, CORS, compression middleware | High |

**Rationale:** Axum is the dominant Rust web framework, built on Tokio. askama compiles templates at build time (no runtime overhead, catches errors at compile time). Alternative: `maud` (macro-based HTML) is faster to write but less designer-friendly. askama wins for maintainability since templates are separate files.

**What NOT to use:**
- `actix-web` — different async runtime (actix), mixing with Tokio process management is painful
- `warp` — less actively maintained, more implicit routing
- `tera` — runtime template parsing, no compile-time checking
- Any JS framework — project constraint: server-rendered HTML only

---

## Process Management

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio::process` | (part of tokio) | Async child process spawning, stdin/stdout/stderr pipes | High |
| `tokio::signal` | (part of tokio) | Signal handling (SIGTERM, SIGINT, SIGHUP) | High |
| `nix` | 0.29.x | Unix-specific process management (process groups, SIGKILL, waitpid) | Medium-High |

**Rationale:** `tokio::process::Command` is the async equivalent of `std::process::Command`. `nix` provides low-level Unix APIs needed for proper process group management (kill entire process trees, not just the direct child). On Windows, `tokio::process` handles WinAPI process termination.

**What NOT to use:**
- `duct` — synchronous, doesn't integrate with Tokio
- `subprocess` — also synchronous
- Raw `libc` — `nix` is the safe wrapper

---

## Daemon Mode

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `daemonize` | 0.5.x | Fork-to-background on Unix (setsid, close fds, redirect stdio) | Medium |
| `tokio::net::UnixListener` | (part of tokio) | IPC socket for CLI-to-daemon communication | High |

**Rationale:** Daemon mode needs: fork, setsid, close stdin/stdout/stderr, write PID file. `daemonize` crate handles the standard Unix double-fork pattern. CLI commands (`status`, `stop`, `logs`) connect to the daemon via a Unix domain socket at a known path (`~/.mcp-hub/mcp-hub.sock`). On Windows, daemon mode can use a named pipe or TCP localhost.

**Alternative considered:** Skip daemonize crate, just use `nix::unistd::fork()` directly — more control but more boilerplate. `daemonize` is small and well-tested.

---

## MCP Protocol / JSON-RPC

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `serde_json` | (already included) | JSON-RPC 2.0 message construction/parsing | High |
| No MCP SDK needed | — | MCP over stdio is simple JSON-RPC; a thin custom client is sufficient | High |

**Rationale:** The MCP protocol over stdio is JSON-RPC 2.0 — send a JSON object to stdin, read a JSON object from stdout. The messages needed are:
- `initialize` (handshake)
- `tools/list`, `resources/list`, `prompts/list` (introspection)
- `ping` (health check)

This is ~100 lines of Rust. An SDK would be overkill and would add a dependency that may not track protocol changes fast enough.

**What NOT to use:**
- `jsonrpc-core` — server-oriented, not client; adds complexity for no benefit
- Any MCP SDK crate — too immature, unnecessary for 4 RPC calls

---

## Logging & Observability

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tracing` | 0.1.x | Structured logging framework | High |
| `tracing-subscriber` | 0.3.x | Log formatting, filtering, output | High |

**Rationale:** `tracing` is the standard Rust logging framework, designed for async (spans work across await points). Used for mcp-hub's own logs, not for managed server logs (those are captured from child process pipes).

**What NOT to use:**
- `log` + `env_logger` — older, no span support, worse async story
- `slog` — less ecosystem support than tracing

---

## Testing

| Crate | Version | Purpose | Confidence |
|-------|---------|---------|------------|
| `tokio::test` | (part of tokio) | Async test runtime | High |
| `assert_cmd` | 2.x | CLI integration testing (run binary, check stdout/exit code) | High |
| `predicates` | 3.x | Fluent assertions for assert_cmd | High |
| `tempfile` | 3.x | Temporary directories for test configs/logs | High |
| `portpicker` | 0.1.x | Find free ports for test web servers | Medium |

---

## Build & Distribution

| Tool | Purpose | Confidence |
|------|---------|------------|
| `cargo` | Build system | High |
| `cross` | Cross-compilation for Linux/Windows from macOS | High |
| GitHub Actions + self-hosted runners | CI/CD | High |
| `cargo-dist` or manual | Binary releases + Homebrew formula generation | Medium |

**Rationale:** `cross` uses Docker to cross-compile for different targets. `cargo-dist` can auto-generate Homebrew formulae and GitHub Release artifacts, but manual release scripts give more control. Evaluate during release phase.

---

## Summary: Cargo.toml Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
anyhow = "1"
thiserror = "2"
axum = "0.8"
askama = "0.12"
tower-http = { version = "0.6", features = ["fs", "cors"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
nix = { version = "0.29", features = ["signal", "process"] }
daemonize = "0.5"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
portpicker = "0.1"
tokio-test = "0.4"
```

---

*Researched: 2026-04-02*
