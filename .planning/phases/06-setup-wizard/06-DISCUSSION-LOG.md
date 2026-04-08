# Phase 6: Setup Wizard - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-08
**Phase:** 06-setup-wizard
**Areas discussed:** Prompt library & flow, Config file targeting, Validation & error recovery, TOML output formatting

---

## Prompt library & flow

| Option | Description | Selected |
|--------|-------------|----------|
| dialoguer | Most popular Rust TUI prompt library. 10M+ downloads. | ✓ |
| inquire | Newer alternative. Built-in validation, custom themes. | |
| You decide | Claude's discretion. | |

**User's choice:** dialoguer
**Notes:** Clean API, good defaults, crossterm backend.

| Option | Description | Selected |
|--------|-------------|----------|
| Essential only | Name → Command → Args → Transport. Done. | ✓ |
| Full wizard | All ServerConfig fields including env vars, env_file, cwd. | |
| Minimal + env | Essential plus env vars loop. | |

**User's choice:** Essential only
**Notes:** Env vars, cwd, env_file added by editing TOML directly.

---

## Config file targeting

| Option | Description | Selected |
|--------|-------------|----------|
| Local ./mcp-hub.toml | Always writes to current directory. Creates if missing. | ✓ |
| Ask the user | Prompt for local or global location. | |
| Respect --config flag | Use -c flag path, default to local. | |

**User's choice:** Local ./mcp-hub.toml
**Notes:** Simple, predictable, matches project-level config patterns.

---

## Validation & error recovery

| Option | Description | Selected |
|--------|-------------|----------|
| Re-prompt with error | Show existing names, ask for different name. Loop. | ✓ |
| Offer to overwrite | Ask to overwrite existing entry. Risky. | |

**User's choice:** Re-prompt with error
**Notes:** Per SC-4: duplicate names produce error prompt.

| Option | Description | Selected |
|--------|-------------|----------|
| No validation | Accept any command string. | ✓ |
| Warn but allow | Check PATH, print warning if not found. | |

**User's choice:** No validation
**Notes:** Users may set up config before installing MCP server binary.

---

## TOML output formatting

| Option | Description | Selected |
|--------|-------------|----------|
| Append raw block | String manipulation, preserves existing formatting. | ✓ |
| Serde round-trip | Deserialize/serialize. Destroys comments. | |
| You decide | Claude's discretion. | |

**User's choice:** Append raw block
**Notes:** Preserves comments, custom formatting, field ordering.

---

## Claude's Discretion

- Success message format
- Args input format (space vs comma separated)
- Malformed TOML handling
- dialoguer feature flags
- Internal module structure

## Deferred Ideas

None — discussion stayed within phase scope
