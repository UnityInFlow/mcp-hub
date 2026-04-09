# mcp-hub

PM2 for MCP servers -- manage, monitor, and configure your MCP servers from a single binary.

## Features

- **TOML config** -- define all MCP servers in one file (name, command, args, env)
- **Process lifecycle** -- start, stop, restart, and status commands for all servers
- **Health monitoring** -- MCP ping checks with configurable interval; Healthy/Degraded/Failed states
- **Auto-restart** -- exponential backoff on crash (1s -> 2s -> 4s -> max 60s)
- **Unified log streaming** -- all server logs interleaved in one view, docker-compose style
- **Web UI dashboard** -- status card grid, tools accordion, SSE log streaming at `http://localhost:3456`
- **Config generation** -- output ready-to-paste mcpServers blocks for Claude Code and Cursor
- **Interactive setup wizard** -- `mcp-hub init` adds a new server with prompts and TOML-safe validation
- **Daemon mode** -- runs in background with Unix socket IPC, PID file, and duplicate prevention

## Installation

**Homebrew (macOS):**

```sh
brew install unityinflow/tap/mcp-hub
```

**Pre-built binaries:**

Download from [GitHub Releases](https://github.com/UnityInFlow/mcp-hub/releases) for macOS (arm64/x86_64), Linux (x86_64/aarch64), and Windows.

**From source:**

```sh
cargo install mcp-server-hub
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
| `mcp-hub status` | Show status of all servers (daemon mode) |
| `mcp-hub logs` | Show recent logs from all servers |
| `mcp-hub logs --server <name>` | Show logs for a specific server |
| `mcp-hub reload` | Reload config and apply changes without restart |
| `mcp-hub init` | Interactive wizard to add a new server |
| `mcp-hub gen-config --format claude` | Generate Claude Code mcpServers JSON block |
| `mcp-hub gen-config --format cursor` | Generate Cursor MCP config snippet |

## Configuration

Full server block reference:

```toml
[hub]
web_port = 3456          # Web UI port (default: 3456)

[servers.my-server]
command = "npx"          # Executable to run
args = ["-y", "@scope/server"]  # Arguments
env = { API_KEY = "${MY_API_KEY}" }  # Environment variables (supports ${VAR} expansion)

[servers.my-server.health]
interval_secs = 30       # How often to send MCP ping (default: 30)

[servers.my-server.restart]
max_retries = 5          # Max restart attempts before marking Fatal (default: 5)
```

## Web UI

The web UI is served at `http://localhost:3456` by default (configurable via `[hub] web_port`). It shows:

- Live server status cards with health state
- MCP tool browser with per-server tool listings
- Real-time log viewer with SSE streaming and per-server filtering

## License

MIT -- see [LICENSE](LICENSE)
