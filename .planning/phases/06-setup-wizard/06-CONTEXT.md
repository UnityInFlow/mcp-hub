# Phase 6: Setup Wizard - Context

**Gathered:** 2026-04-08
**Status:** Ready for planning

<domain>
## Phase Boundary

Provide an interactive `mcp-hub init` wizard that prompts for server name, command, args, and transport, then appends the new server entry to `./mcp-hub.toml` (creating the file if needed). Pure CLI feature — no daemon, no web UI changes.

Requirements: WIZ-01, WIZ-02, WIZ-03.

</domain>

<decisions>
## Implementation Decisions

### Prompt library & flow
- **D-01:** Use `dialoguer` crate for interactive terminal prompts (Input, Select). Most popular Rust TUI prompt library, crossterm backend, clean API.
- **D-02:** Essential fields only in the prompt sequence: Name (required) → Command (required) → Args (optional, comma-separated, Enter to skip) → Transport (Select: stdio/http, default stdio). Done.
- **D-03:** Env vars, cwd, env_file, and other optional ServerConfig fields are NOT prompted. Users add these by editing the TOML file directly after init creates the entry.

### Config file targeting
- **D-04:** Always writes to `./mcp-hub.toml` in the current directory. Does NOT use the global `~/.config/mcp-hub/` path.
- **D-05:** If `./mcp-hub.toml` doesn't exist, create it with the new server as the first entry. If it exists, append to it.

### Validation & error recovery
- **D-06:** Duplicate server name: show error listing existing server names, re-prompt until the user enters a unique name. Per SC-4.
- **D-07:** No command path validation. Accept any command string without checking if it exists in PATH. Users may set up config before installing the MCP server binary.
- **D-08:** Name must not be empty. Command must not be empty. These are the only hard validation rules (same as `validate_config` in config.rs).

### TOML output formatting
- **D-09:** Append a hand-crafted TOML block to the end of the file using string manipulation (NOT serde round-trip). Preserves existing comments, formatting, and whitespace.
- **D-10:** New block format: blank line separator, then `[servers.<name>]` header, then fields (command, args if non-empty, transport if not stdio). Omit default values.

### Claude's Discretion
- Whether to print a summary after writing (e.g., "Added 'github' to ./mcp-hub.toml")
- Whether args prompt accepts space-separated or comma-separated input
- How to handle the edge case where the file exists but is malformed TOML
- Whether to add `dialoguer` feature flags (e.g., `console` backend) or use defaults
- Internal module structure for init code (src/init.rs or inline in main.rs)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` — Full feature spec, setup wizard section
- `.planning/PROJECT.md` — Project vision, validated requirements, constraints

### Requirements
- `.planning/REQUIREMENTS.md` — WIZ-01, WIZ-02, WIZ-03 definitions

### Prior phase context
- `.planning/phases/01-config-process-supervisor/01-CONTEXT.md` — D-01 through D-06 (config shape, ServerConfig fields)

### Existing code (key files for Phase 6)
- `src/config.rs` — `ServerConfig`, `HubConfig`, `validate_config`, `find_and_load_config` (config shape and validation rules)
- `src/cli.rs` — `Commands` enum (add `Init` variant), clap derive pattern
- `src/main.rs` — Command dispatch (add `Commands::Init` match arm)
- `Cargo.toml` — Add `dialoguer` dependency

### Ecosystem constraints
- `CLAUDE.md` — Rust coding standards, no unwrap(), cargo clippy/fmt

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `ServerConfig` (config.rs) — Target struct whose fields define the wizard's prompt sequence
- `validate_config` (config.rs) — Reuse for validating the new entry before writing
- `load_config` (config.rs) — Load existing config to check for duplicate names
- clap `Commands` enum (cli.rs) — Add `Init` variant following established pattern

### Established Patterns
- clap derive for CLI subcommands (cli.rs)
- anyhow for error handling in binary
- TOML via `toml` crate for reading (but NOT for writing in this phase — string append)
- tracing for warnings/info messages

### Integration Points
- `Commands` enum in cli.rs — add `Init` variant (no args, interactive)
- `main.rs` command dispatch — add match arm for `Commands::Init`
- New `src/init.rs` module — interactive wizard logic, TOML append
- `Cargo.toml` — add `dialoguer` dependency

</code_context>

<specifics>
## Specific Ideas

- The wizard should feel quick and lightweight — 4 prompts and done, not a lengthy interview
- Success message should show the file path and the server name added
- Entering an empty name or command should re-prompt (not error out and exit)
- The appended TOML block should look like a human wrote it — clean, minimal, no unnecessary fields

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 06-setup-wizard*
*Context gathered: 2026-04-08*
