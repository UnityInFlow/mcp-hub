---
phase: 06-setup-wizard
plan: "01"
subsystem: init-wizard
tags: [rust, dialoguer, toml, cli, interactive]
dependency_graph:
  requires: []
  provides: [mcp-hub-init-wizard]
  affects: [src/cli.rs, src/main.rs, src/init.rs, Cargo.toml]
tech_stack:
  added: [dialoguer 0.11]
  patterns: [dialoguer Input/Select, hand-crafted TOML string append, path-parameterised helpers for testability]
key_files:
  created:
    - src/init.rs
    - tests/init_wizard.rs
  modified:
    - Cargo.toml
    - src/cli.rs
    - src/main.rs
    - src/lib.rs
decisions:
  - dialoguer 0.11 chosen per D-01 for interactive terminal prompts (Input, Select with crossterm backend)
  - Hand-crafted TOML string append (not serde round-trip) to preserve existing comments and whitespace (D-09)
  - write_server_entry_to / existing_server_names_from path-parameterised helpers enable integration tests without process-level stdin mocking
  - is_valid_server_name rejects names with TOML-unsafe characters (T-06-04 mitigation)
  - Default transport "stdio" omitted from generated TOML block to keep config minimal (D-10)
metrics:
  duration: "~15 minutes"
  completed: "2026-04-08T20:58:15Z"
  tasks_completed: 2
  files_changed: 6
requirements_covered: [WIZ-01, WIZ-02, WIZ-03]
---

# Phase 6 Plan 01: Setup Wizard — Init Command Summary

Interactive `mcp-hub init` wizard using dialoguer crate: 4-step prompt sequence (name, command, args, transport) writes a clean TOML server block to `./mcp-hub.toml` (create or append), with duplicate-name rejection and TOML-safe name validation.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Create init module with wizard logic and unit tests | 537dc90 | src/init.rs, src/cli.rs, src/main.rs, src/lib.rs, Cargo.toml |
| 2 | Integration tests for file create/append/format/names | 5e1e365 | tests/init_wizard.rs |

## What Was Built

**`src/init.rs`** — Full wizard module with:
- `run_init_wizard()` — main entry point, prompts user, writes TOML
- `format_toml_block(name, command, args, transport)` — hand-crafted TOML string; omits args when empty, omits transport when "stdio"
- `write_server_entry(toml_block)` — appends to `./mcp-hub.toml` or creates it
- `write_server_entry_to(path, toml_block)` — path-parameterised version for tests
- `existing_server_names()` — reads `./mcp-hub.toml`, returns server name list
- `existing_server_names_from(path)` — path-parameterised version for tests
- `is_valid_server_name(name)` — rejects empty names and TOML-unsafe characters

**`src/cli.rs`** — `Commands::Init` variant added.

**`src/main.rs`** — `Commands::Init => init::run_init_wizard()` dispatch arm added.

**Tests:**
- 11 unit tests in `src/init.rs` (format_toml_block: 6, existing_server_names: 3, is_valid_server_name: 2)
- 8 integration tests in `tests/init_wizard.rs` (new file, append, existing names, format corners, no leading blank)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Security] Added TOML-safe name validation**
- **Found during:** Task 1 implementation
- **Issue:** Threat model T-06-04 required rejecting names with TOML-unsafe characters (quotes, brackets, newlines). The plan mentioned this in the threat register but did not specify the implementation.
- **Fix:** Added `is_valid_server_name()` that only allows A-Za-z0-9, `-`, `_` — characters valid in TOML bare keys. Used in the dialoguer `validate_with` closure for the name prompt.
- **Files modified:** src/init.rs
- **Commit:** 537dc90

**2. [Rule 2 - Testability] Added `_from` / `_to` path-parameterised variants**
- **Found during:** Task 2 — integration tests cannot control cwd for parallel test execution
- **Issue:** Plan itself anticipated this and described the `_from` / `_to` approach. Implemented as specified.
- **Fix:** `write_server_entry_to(path, block)` and `existing_server_names_from(path)` as public functions; original `write_server_entry` and `existing_server_names` delegate to them with the hardcoded `./mcp-hub.toml` path.
- **Files modified:** src/init.rs
- **Commit:** 537dc90

## Threat Surface Scan

No new network endpoints, auth paths, or file access patterns beyond what the plan's threat model documented. The wizard writes only to `./mcp-hub.toml` in the current directory. The T-06-04 TOML injection mitigation (name validation) was applied.

## Self-Check: PASSED

- src/init.rs: FOUND
- tests/init_wizard.rs: FOUND
- Commands::Init in src/cli.rs: FOUND
- Commands::Init dispatch in src/main.rs: FOUND
- dialoguer in Cargo.toml: FOUND
- Commit 537dc90: FOUND
- Commit 5e1e365: FOUND
- cargo build: PASS
- cargo clippy -D warnings: PASS
- cargo fmt --check: PASS
- cargo test --lib init (11 tests): PASS
- cargo test --test init_wizard (8 tests): PASS
