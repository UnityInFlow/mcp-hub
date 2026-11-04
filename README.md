# mcp-hub

PM2 for MCP servers: one config file, one command, one log stream.

mcp-hub is a single Rust binary that runs your local (stdio) MCP servers as
supervised child processes, restarts them when they crash, shows their health
and logs in one place, and generates the client config for Claude Code, Cursor,
Codex CLI and Gemini CLI from the same server list.

<!-- demo GIF: docs/demo/demo.gif (see docs/demo/README.md) -->

```sh
cargo binstall mcp-server-hub        # installs the `mcp-hub` binary
mcp-hub init                         # add a server to ./mcp-hub.toml
mcp-hub start --daemon               # run everything in the background
mcp-hub status                       # state, health, PID, uptime, restarts
mcp-hub logs --follow                # all servers' logs, interleaved
mcp-hub gen-config --format claude   # paste into .mcp.json
```

## What it does

- **Supervises processes.** Reads server definitions (`command`, `args`, `env`) from a TOML file, and starts, stops and restarts them. Each server runs in its own process group, so stopping it also kills the child processes it spawned.
- **Restarts on crash.** Exponential backoff (1s, 2s, 4s, up to 60s) with a retry limit (`max_retries`); a server that keeps failing is marked `fatal`.
- **Checks health.** Sends MCP `ping` requests at a configurable interval and reports each server as Healthy, Degraded or Failed.
- **Introspects servers.** Lists each server's tools, resources and prompts over the MCP protocol.
- **Streams logs.** `mcp-hub logs --follow` interleaves the stderr of all servers (or one, with `--server`).
- **Serves a local web UI.** Status, a tools browser and live logs at `http://127.0.0.1:3456`, plus a `/health` JSON endpoint that returns HTTP 503 when a server has failed.
- **Generates client config.** `mcp-hub gen-config --format claude|cursor|codex|gemini` prints the config document on stdout (notes go to stderr). `${VAR}` and `${VAR:-default}` are expanded in `env` and `args`, and `env_file` is supported.
- **Runs as a daemon.** `start --daemon` waits until the servers and sockets are up and reports startup errors; its output goes to `daemon.log` next to the control socket (or `--log-file`). `mcp-hub reload` applies config changes and reports how many servers were added, removed and changed.

## What it does not do

- **It is not an MCP proxy or gateway.** It does not sit between your client and your servers and does not forward MCP traffic. `gen-config` writes each server's own `command`, `args` and `env` into the client config, so the client starts its own copy of the server. mcp-hub's copy is separate: it is the one that is supervised, health-checked, logged and shown in the web UI. If you want a single endpoint that aggregates many servers, use an MCP proxy or gateway instead.
- **It does not install MCP servers.** `command` is whatever you would type in a terminal (`npx ...`, `uvx ...`, a local binary).
- **It does not manage remote machines.** Everything runs on the local host, and the control socket is local.
- **The web UI has no authentication.** It only binds to `127.0.0.1`; do not expose that port to other machines.
- **One daemon per user.** The control socket lives in a single per-user location (`~/.config/mcp-hub/` on Linux), so you cannot run several independent daemons for different projects ([#21](https://github.com/UnityInFlow/mcp-hub/issues/21)).
- **No Windows support yet.** Linux and macOS only ([#30](https://github.com/UnityInFlow/mcp-hub/issues/30)).
- **No Homebrew formula yet** ([#7](https://github.com/UnityInFlow/mcp-hub/issues/7)); use `cargo binstall` or `cargo install`.

## Naming

The crate on crates.io is **`mcp-server-hub`** (the name `mcp-hub` belongs to an unrelated project). The repository and the installed binary are both **`mcp-hub`**. Install with `mcp-server-hub`, run `mcp-hub`.

## Installation

**Prebuilt binary (fastest) — `cargo binstall`:**

```sh
cargo binstall mcp-server-hub      # downloads a prebuilt mcp-hub binary, no compile
```

**From source — `cargo install`:**

```sh
cargo install mcp-server-hub       # compiles and installs the mcp-hub binary
```

**Direct download:**

Grab `mcp-server-hub-<target>-v<version>.tgz` (Linux or macOS) from
[GitHub Releases](https://github.com/UnityInFlow/mcp-hub/releases), extract the
`mcp-hub` binary, and put it on your `PATH`.

Prebuilt targets built by the release workflow (v0.1.1 shipped Linux only; macOS archives start with the next release):

| Platform | Target triple | Asset |
|----------|---------------|-------|
| Linux x86_64 (glibc) | `x86_64-unknown-linux-gnu` | `mcp-server-hub-x86_64-unknown-linux-gnu-v<version>.tgz` |
| Linux aarch64 (glibc) | `aarch64-unknown-linux-gnu` | `mcp-server-hub-aarch64-unknown-linux-gnu-v<version>.tgz` |
| Linux x86_64 (musl) | `x86_64-unknown-linux-musl` | `mcp-server-hub-x86_64-unknown-linux-musl-v<version>.tgz` |
| Linux aarch64 (musl) | `aarch64-unknown-linux-musl` | `mcp-server-hub-aarch64-unknown-linux-musl-v<version>.tgz` |
| macOS Apple Silicon (aarch64) | `aarch64-apple-darwin` | `mcp-server-hub-aarch64-apple-darwin-v<version>.tgz` |
| macOS Intel (x86_64) | `x86_64-apple-darwin` | `mcp-server-hub-x86_64-apple-darwin-v<version>.tgz` |

> **Windows** is not supported yet ([#30](https://github.com/UnityInFlow/mcp-hub/issues/30)).
> The daemon, Unix-socket control channel, web UI and signal handling are Unix-only.

After installing, verify with:

```sh
mcp-hub --version
```

## Quick Start

Create a `mcp-hub.toml` in your project directory:

```toml
[hub]
web_port = 3456

[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "${GITHUB_TOKEN}" }
```

Then start all servers:

```sh
mcp-hub start
```

## Commands

| Command | Description |
|---------|-------------|
| `mcp-hub start` | Start all servers in foreground mode |
| `mcp-hub start --daemon` | Start all servers as a background daemon |
| `mcp-hub stop` | Stop the running daemon |
| `mcp-hub restart <name>` | Restart a specific server |
| `mcp-hub status` | Show status of all servers as a table (daemon mode); `--json` for raw JSON |
| `mcp-hub logs` | Show recent logs from all servers |
| `mcp-hub logs --server <name>` | Show logs for a specific server |
| `mcp-hub logs --follow` | Stream live logs from all servers (recent lines first; Ctrl+C to stop) |
| `mcp-hub logs --follow --server <name>` | Stream live logs for one server |
| `mcp-hub reload` | Reload config and apply changes without restart |
| `mcp-hub init` | Interactive wizard to add a new server |
| `mcp-hub gen-config --format claude` | Generate Claude Code mcpServers JSON block |
| `mcp-hub gen-config --format cursor` | Generate Cursor MCP config snippet |
| `mcp-hub gen-config --format codex` | Generate Codex CLI `[mcp_servers.<name>]` TOML |
| `mcp-hub gen-config --format gemini` | Generate Gemini CLI `mcpServers` JSON block |

**Codex CLI / Gemini CLI.** `--format codex` prints TOML to append to `~/.codex/config.toml` (or `.codex/config.toml` in a trusted project); `--format gemini` prints JSON to merge into `~/.gemini/settings.json` (user) or `.gemini/settings.json` (project). As with the other formats, stdout holds only the config document and the destination hint and warnings go to stderr, so `mcp-hub gen-config -f codex >> ~/.codex/config.toml` works. `cwd` is emitted for these two formats when a server sets one. `transport = "http"` servers are skipped with a warning.

## Configuration

Full server block reference:

```toml
[hub]
web_port = 3456          # Web UI port (default: 3456)

[servers.my-server]
command = "npx"          # Executable to run (required, must not be empty)
args = ["-y", "@scope/server"]  # Arguments passed to the executable (default: [])
env = { LOG_LEVEL = "info" }  # Inline environment variables; ${VAR} is expanded (default: none)
env_file = ".env"        # Path to a KEY=VALUE file (# comments ok); its values override `env` (default: none)
transport = "stdio"      # "stdio" or "http" (default: "stdio")
cwd = "/path/to/project" # Working directory for the process (default: inherited)
health_check_interval = 30  # Seconds between MCP pings (default: 30)
max_retries = 10         # Consecutive crashes before marking Fatal (default: 10)
restart_delay = 1        # Base restart backoff delay in seconds (default: 1)
```

### Environment variable expansion

`env` values and `args` may reference the environment `mcp-hub` itself runs in, so secrets can stay out of the config file:

- `${NAME}` is replaced by the value of `NAME`. If `NAME` is not set, loading the config fails with an error naming the server, the field and the variable.
- `${NAME:-default}` uses `default` (which may be empty) when `NAME` is unset or empty.
- `$${` is an escape and produces a literal `${`.
- A lone `$` or `$NAME` without braces is left as written. `NAME` must match `[A-Za-z_][A-Za-z0-9_]*`, and an unterminated `${` is an error.

`command`, `cwd` and `env_file` are not expanded, and values read from `env_file` are always literal; `env_file` remains the alternative for secrets you would rather keep in a separate file. `mcp-hub gen-config` writes the expanded values (that is what ends up in `.mcp.json`), and prints a warning on stderr when one of them came from a variable whose name contains `TOKEN`, `SECRET`, `KEY` or `PASSWORD`.

## Web UI

The web UI is served at `http://127.0.0.1:3456` by default (configurable via `[hub] web_port`). It listens on the loopback interface only and has no authentication. It shows:

- Live server status cards with health state
- MCP tool browser with per-server tool listings
- Real-time log viewer with SSE streaming and per-server filtering

It also serves `GET /health`, a JSON summary that returns HTTP 503 when a server has failed.

## License

MIT -- see [LICENSE](LICENSE)
