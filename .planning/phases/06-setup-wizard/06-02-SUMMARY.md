---
phase: 06-setup-wizard
plan: "02"
subsystem: init-wizard
tags: [rust, toml, security, correctness, tdd]
dependency_graph:
  requires: [06-01]
  provides: [toml-escape-correctness, ascii-only-server-names, toctou-fix]
  affects: [src/init.rs, tests/init_wizard.rs]
tech_stack:
  added: []
  patterns: [toml_escape helper, ASCII char range matching, read-before-open TOCTOU fix]
key_files:
  created: []
  modified:
    - src/init.rs
    - tests/init_wizard.rs
decisions:
  - toml_escape uses two sequential replace() calls (backslash first, then quote) — order matters to avoid double-escaping
  - ASCII-only validation uses matches! macro with explicit char ranges instead of is_alphanumeric() to exclude all Unicode
  - TOCTOU fix reads file to string before opening handle for append — simple and correct for single-user local tool
metrics:
  duration: "~20 minutes"
  completed: "2026-04-08T21:24:38Z"
  tasks_completed: 2
  files_changed: 2
requirements_covered: [WIZ-01, WIZ-02, WIZ-03]
---

# Phase 6 Plan 02: Setup Wizard Gap Closure Summary

TOML injection prevention via `toml_escape()` helper applied to command/args/transport; ASCII-only server name validation replacing Unicode-accepting `is_alphanumeric()`; TOCTOU fix in `write_server_entry_to` reads file content before opening append handle.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Add toml_escape, fix is_valid_server_name, fix write_server_entry_to TOCTOU | 47d4f5f | src/init.rs |
| 2 | Add TOML escaping roundtrip integration tests | 1f86c61 | tests/init_wizard.rs |

## What Was Built

**`src/init.rs` — three correctness fixes:**

1. **`toml_escape(s: &str) -> String`** — new private helper that escapes `\` to `\\` and `"` to `\"`. Applied to `command`, each element of `args`, and `transport` in `format_toml_block`. Prevents TOML injection (CR-01 / T-06-04).

2. **`is_valid_server_name`** — replaced `c.is_alphanumeric()` with `matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_')`. Non-ASCII Unicode characters (accented letters, CJK, combining codepoints) are now rejected (CR-02 / T-06-05).

3. **`write_server_entry_to`** — reordered the append branch: `read_to_string` now executes before `OpenOptions::new().append(true)`, eliminating the TOCTOU window between checking the trailing newline and writing (WR-02 / T-06-06).

**New unit tests in `src/init.rs` (5 new, 16 total):**
- `toml_escape_plain_string_unchanged`
- `toml_escape_backslash`
- `toml_escape_double_quote`
- `toml_escape_both`
- `rejects_unicode_server_names`

**New integration tests in `tests/init_wizard.rs` (4 new, 12 total):**
- `test_format_block_escapes_backslash_and_quote` — Windows path command roundtrips through TOML parse
- `test_format_block_escapes_double_quote_in_command` — quoted command roundtrips
- `test_format_block_escapes_args_with_special_chars` — args with backslash and quote roundtrip
- `test_format_block_windows_path_full_roundtrip` — full write/read/parse cycle with Windows paths in both command and args

## Deviations from Plan

None — plan executed exactly as written. All three patches (CR-01, CR-02, WR-02) applied as specified in the plan's `<action>` blocks.

## Overrides Applied

Per plan frontmatter `overrides`:
- **WIZ-02 env vars prompt** — deliberately excluded. Per decision D-03 ("Essential fields only"), env vars are added by editing TOML directly after `mcp-hub init` creates the entry. This is an intentional scope boundary, not a gap.

## Threat Surface Scan

All fixes directly mitigate threats already in the plan's threat model:
- T-06-04 (TOML injection via unescaped command/args) — mitigated by `toml_escape`
- T-06-05 (Unicode in TOML bare keys) — mitigated by ASCII-only matching
- T-06-06 (TOCTOU in write_server_entry_to) — accepted risk, fixed for correctness

No new network endpoints, auth paths, file access patterns, or schema changes introduced.

## Self-Check: PASSED

- `fn toml_escape` in src/init.rs: FOUND
- `toml_escape(command)` in src/init.rs: FOUND
- `toml_escape(a)` in src/init.rs: FOUND
- `'A'..='Z'` ASCII range in src/init.rs: FOUND
- `is_alphanumeric` count in src/init.rs: 0 (removed)
- `read_to_string` before `OpenOptions` in exists branch: CONFIRMED (lines 106 vs 109)
- `test_format_block_escapes_backslash_and_quote` in tests/init_wizard.rs: FOUND
- `test_format_block_escapes_double_quote_in_command` in tests/init_wizard.rs: FOUND
- `test_format_block_escapes_args_with_special_chars` in tests/init_wizard.rs: FOUND
- `test_format_block_windows_path_full_roundtrip` in tests/init_wizard.rs: FOUND
- Commit 47d4f5f: FOUND
- Commit 1f86c61: FOUND
- `cargo test --lib init` (16 tests): PASS
- `cargo test --test init_wizard` (12 tests): PASS
- `cargo clippy -- -D warnings`: PASS
- `cargo fmt --check`: PASS
