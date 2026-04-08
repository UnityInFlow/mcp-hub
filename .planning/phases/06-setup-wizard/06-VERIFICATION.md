---
phase: 06-setup-wizard
verified: 2026-04-08T21:30:00Z
status: human_needed
score: 4/4 must-haves verified
overrides_applied: 1
overrides:
  - must_have: "mcp-hub init launches an interactive prompt sequence asking for name, command, args, env vars, and transport type"
    reason: "Env vars deliberately excluded per discussion D-03 — user chose 'Essential fields only'. Env vars are added by editing TOML directly after init creates the entry. This is an intentional scope decision recorded in 06-CONTEXT.md, not a bug."
    accepted_by: "jirihermann"
    accepted_at: "2026-04-08T21:24:38Z"
    source: "06-02-PLAN.md frontmatter overrides / 06-02-SUMMARY.md"
re_verification:
  previous_status: gaps_found
  previous_score: 3/4
  gaps_closed:
    - "Generated TOML block is safe for all user inputs (special characters in command/args) — toml_escape() helper added and applied"
    - "Server names with non-ASCII Unicode characters (accented letters, CJK) now rejected by ASCII-only is_valid_server_name"
    - "write_server_entry_to reads file before opening for append (TOCTOU eliminated)"
  gaps_remaining: []
  regressions: []
human_verification:
  - test: "Run `mcp-hub init` in a fresh terminal directory. Enter: name=github, command=npx, args=@anthropic/mcp-github, transport=stdio. Verify ./mcp-hub.toml is created with [servers.github], command = \"npx\", args = [\"@anthropic/mcp-github\"]. No transport line should appear."
    expected: "File created at ./mcp-hub.toml with correct content. Success message: Added 'github' to ./mcp-hub.toml."
    why_human: "dialoguer checks if stdin is a TTY and refuses interactive prompts in non-TTY environments. Cannot pipe stdin in CI."
  - test: "Run `mcp-hub init` in a directory containing an existing mcp-hub.toml with server 'github'. At the name prompt, type 'github'. Verify the wizard re-prompts with an error listing existing servers."
    expected: "Error message: \"Name 'github' already exists. Existing servers: github\". Prompt repeats — wizard does not exit."
    why_human: "Requires interactive TTY to exercise dialoguer validate_with re-prompt loop."
  - test: "At the name prompt press Enter (empty input). At the command prompt press Enter."
    expected: "Wizard re-prompts with 'Server name must not be empty' then 'Command must not be empty'. Does not crash or exit."
    why_human: "Requires interactive TTY input simulation."
---

# Phase 6: Setup Wizard Verification Report

**Phase Goal:** Provide an interactive `mcp-hub init` wizard to add new servers to the TOML config without manual editing.
**Verified:** 2026-04-08T21:30:00Z
**Status:** human_needed
**Re-verification:** Yes — after gap closure plans 06-02 fixed TOML escaping, Unicode server names, and TOCTOU I/O.

---

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | `mcp-hub init` launches an interactive prompt sequence asking for name, command, args, env vars, and transport type | PASSED (override) | 4-step wizard present (name, command, args, transport). Env vars excluded per D-03 intentional scope decision. Override accepted by jirihermann per 06-02-PLAN.md. |
| 2 | Running `mcp-hub init` in a directory with an existing `mcp-hub.toml` appends without overwriting | VERIFIED | `write_server_entry_to()` opens in append mode; `test_append_to_existing_file` verifies both servers present after append, parsed as valid TOML. |
| 3 | Running `mcp-hub init` in a directory with no config file creates a new `mcp-hub.toml` | VERIFIED | `write_server_entry_to()` branches on `path.exists()`; new file trimmed of leading newline. `test_format_and_write_new_file` passes; file parses as valid TOML. |
| 4 | Duplicate server names are rejected with a clear error and re-prompt | VERIFIED | `existing_server_names()` called before prompts; dialoguer `validate_with` closure returns `Err("Name 'X' already exists. Existing servers: A, B")` causing automatic re-prompt. |

**Score:** 4/4 must-haves verified (1 via accepted override, 3 programmatically)

### Gaps Closed Since Previous Verification

| Gap | Previous Status | Current Status | Evidence |
|-----|----------------|----------------|----------|
| TOML escaping: `format_toml_block` unescaped `"` and `\` in command/args | FAILED (blocker) | VERIFIED | `fn toml_escape` at `src/init.rs:8`. Applied to command (line 73), each arg element (line 79), transport (line 85). 4 unit tests + 4 roundtrip integration tests pass (12 total). |
| Unicode server names accepted by `is_alphanumeric()` | WARNING | VERIFIED | `is_valid_server_name` now uses `matches!(c, 'A'..='Z' \| 'a'..='z' \| '0'..='9' \| '-' \| '_')`. `is_alphanumeric` count in file: 0. `rejects_unicode_server_names` test passes. |
| TOCTOU: `write_server_entry_to` opened for append before reading existing content | WARNING | VERIFIED | `read_to_string` at line 106 executes before `OpenOptions::new().append(true)` at line 109. Both in the `path.exists()` branch. |
| WIZ-02 env vars prompt — not implemented | PARTIAL | PASSED (override) | Intentional per D-03. Override recorded in 06-02-PLAN.md frontmatter and carried forward here. |

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `src/init.rs` | Interactive wizard logic: prompt sequence, TOML generation, file write (min 80 lines) | VERIFIED | 376 lines. Contains `run_init_wizard`, `format_toml_block` (with `toml_escape`), `write_server_entry`, `write_server_entry_to`, `existing_server_names`, `existing_server_names_from`, `is_valid_server_name`, `toml_escape`. No `unwrap()` in production paths. |
| `src/cli.rs` | Init variant in Commands enum | VERIFIED | Line 59: `Init,` with doc comment "Interactively add a new MCP server to ./mcp-hub.toml." |
| `src/main.rs` | Commands::Init match arm dispatching to init module | VERIFIED | Line 371: `Commands::Init => init::run_init_wizard()` |
| `Cargo.toml` | dialoguer dependency | VERIFIED | Line 34: `dialoguer = "0.11"` |
| `tests/init_wizard.rs` | Integration tests for TOML file creation and append (min 40 lines) | VERIFIED | 231 lines, 12 integration tests: create, append, existing names, format corners, no-leading-blank, 4 TOML escaping roundtrip tests. |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/main.rs` | `src/init.rs` | `Commands::Init` match arm calls `init::run_init_wizard` | WIRED | Line 371: `Commands::Init => init::run_init_wizard()` |
| `src/init.rs` | `format_toml_block` | `toml_escape` applied to command and each arg | WIRED | `toml_escape(command)` at line 73; `toml_escape(a)` inside map at line 79 |
| `src/cli.rs` | `src/main.rs` | `Commands::Init` variant matched in `async_main` | WIRED | `Init` at cli.rs:59 matched at main.rs:371 |

**Note on `config::load_config` deviation (carried forward from previous):** The plan's key_links listed `config::load_config` as the via-pattern for duplicate detection. The implementation uses `toml::from_str::<toml::Value>` instead — a deliberate safety choice so a malformed config does not block `init`. Functionally equivalent for duplicate detection.

### Data-Flow Trace (Level 4)

Not applicable. The init wizard writes data to the filesystem; it does not render dynamic data from a data source. No data-flow trace required.

### Behavioral Spot-Checks

| Behavior | Command | Result | Status |
|----------|---------|--------|--------|
| `mcp-hub init --help` shows the subcommand | `./target/debug/mcp-hub init --help` | "Interactively add a new MCP server to ./mcp-hub.toml" present | PASS |
| Unit tests pass (16 tests) | `cargo test --lib init` | 16 passed, 0 failed | PASS |
| Integration tests pass (12 tests) | `cargo test --test init_wizard` | 12 passed, 0 failed | PASS |
| `cargo clippy -- -D warnings` | `cargo clippy -- -D warnings` | No issues found | PASS |
| `cargo fmt --check` | `cargo fmt --check` | Clean (exit 0) | PASS |
| `toml_escape` helper exists | `grep "fn toml_escape" src/init.rs` | Found at line 8 | PASS |
| `is_alphanumeric` removed | `grep -c "is_alphanumeric" src/init.rs` | 0 matches | PASS |
| TOCTOU fix: read before append-open | `grep -n "read_to_string" src/init.rs` | Line 106 precedes `OpenOptions` at line 109 | PASS |

### Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|------------|-------------|--------|----------|
| WIZ-01 | 06-01-PLAN.md | User can add a new server interactively with `mcp-hub init` | SATISFIED | `mcp-hub init` subcommand present in CLI, dispatches to `init::run_init_wizard()`, launches dialoguer prompt sequence, writes TOML entry. |
| WIZ-02 | 06-01-PLAN.md | Wizard prompts for name, command, args, env vars, transport type | SATISFIED (override) | Prompts for name, command, args, transport. Env vars excluded per D-03; override accepted by jirihermann. |
| WIZ-03 | 06-01-PLAN.md | Wizard appends to existing TOML config or creates a new one | SATISFIED | `write_server_entry_to()` handles both create and append; `test_format_and_write_new_file` and `test_append_to_existing_file` both pass, each verifying valid TOML output. |

**Orphaned requirements check:** REQUIREMENTS.md maps exactly WIZ-01, WIZ-02, WIZ-03 to Phase 6. All three are accounted for. No orphaned requirements.

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| None | — | — | — | All prior blockers and warnings resolved by 06-02 gap closure. |

No `unwrap()` in production paths. No placeholder or stub patterns. No hardcoded empty data returned to callers.

### Human Verification Required

#### 1. Full Interactive Wizard Flow

**Test:** Run `mcp-hub init` in a fresh terminal directory. Enter: name=`github`, command=`npx`, args=`@anthropic/mcp-github`, transport=`stdio`.
**Expected:** File created at `./mcp-hub.toml` with `[servers.github]`, `command = "npx"`, `args = ["@anthropic/mcp-github"]`. No transport line. Success message: `Added 'github' to ./mcp-hub.toml`.
**Why human:** dialoguer detects non-TTY stdin and refuses interactive prompts. Cannot automate.

#### 2. Duplicate Name Re-Prompt Behavior

**Test:** Run `mcp-hub init` in a directory with an existing `mcp-hub.toml` containing `[servers.github]`. At the name prompt enter `github`.
**Expected:** Error message "Name 'github' already exists. Existing servers: github". Prompt repeats — wizard does not exit.
**Why human:** Requires interactive TTY to exercise the dialoguer `validate_with` re-prompt loop.

#### 3. Empty Name and Empty Command Re-Prompt

**Test:** At the name prompt press Enter (empty input). At the command prompt press Enter.
**Expected:** Wizard re-prompts with "Server name must not be empty" then "Command must not be empty". Does not crash or exit.
**Why human:** Requires interactive TTY input simulation.

---

## Gaps Summary

No gaps remain. All two previous gaps are closed:

- **TOML escaping blocker (CR-01):** `toml_escape()` helper added and applied to `command`, each `args` element, and `transport` in `format_toml_block()`. Four roundtrip integration tests prove correctness end-to-end.
- **WIZ-02 env vars gap:** Accepted as intentional override per D-03. Recorded in 06-02-PLAN.md frontmatter and carried forward here.

The three human verification items above are behavioral (interactive TTY) and cannot be automated. Once the developer completes those manual checks, the phase is fully clear.

---

_Verified: 2026-04-08T21:30:00Z_
_Verifier: Claude (gsd-verifier)_
_Re-verification: Yes — after gap closure plan 06-02_
