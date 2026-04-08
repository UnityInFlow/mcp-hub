---
phase: 06-setup-wizard
reviewed: 2026-04-08T00:00:00Z
depth: standard
files_reviewed: 6
files_reviewed_list:
  - Cargo.toml
  - src/cli.rs
  - src/init.rs
  - src/main.rs
  - src/lib.rs
  - tests/init_wizard.rs
findings:
  critical: 2
  warning: 4
  info: 2
  total: 8
status: issues_found
---

# Phase 06: Code Review Report

**Reviewed:** 2026-04-08
**Depth:** standard
**Files Reviewed:** 6
**Status:** issues_found

## Summary

The setup wizard implementation (`src/init.rs`) is well-structured: the interactive flow is clean, the TOML formatting logic is pure and testable, and the test coverage in both the unit module and `tests/init_wizard.rs` is thorough. The public API is correctly exported through `src/lib.rs`.

Two critical correctness issues exist in `src/init.rs`: user input containing `"` or `\` characters will produce malformed TOML (silently breaking the config file), and the Unicode handling in `is_valid_server_name` is wider than TOML bare key rules permit. A reliability bug in `src/main.rs` holds a `Mutex` across a 2-second sleep in the foreground loop, blocking the concurrent web server. A TOCTOU file-handle ordering issue in `write_server_entry_to` is also worth resolving.

---

## Critical Issues

### CR-01: Unescaped user input produces invalid TOML in `format_toml_block`

**File:** `src/init.rs:64-76`

**Issue:** The `command`, each element of `args`, and `name` are interpolated directly into double-quoted TOML basic strings and bare keys using Rust format macros. TOML basic strings require `"` to be escaped as `\"` and `\` as `\\`. A command such as `node "my server.js"` or a path like `C:\tools\mcp.exe` will produce syntactically invalid TOML. The generated file will then fail to parse on the next `mcp-hub start`, silently breaking the user's config. The `name` interpolation into a bare key is safe because `is_valid_server_name` already guards that, but `command` and `args` accept any non-empty string.

**Fix:** Escape `"` and `\` in any string value before placing it inside double-quoted TOML:

```rust
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn format_toml_block(name: &str, command: &str, args: &[String], transport: &str) -> String {
    let mut block = format!(
        "\n[servers.{name}]\ncommand = \"{}\"\n",
        toml_escape(command)
    );

    if !args.is_empty() {
        let quoted: Vec<String> = args
            .iter()
            .map(|a| format!("\"{}\"", toml_escape(a)))
            .collect();
        block.push_str(&format!("args = [{}]\n", quoted.join(", ")));
    }

    if transport != "stdio" {
        block.push_str(&format!("transport = \"{}\"\n", toml_escape(transport)));
    }

    block
}
```

Also add a test that roundtrips a command with a backslash and a quote through `format_toml_block` → `toml::from_str` and verifies the parsed value equals the original input.

---

### CR-02: `is_valid_server_name` allows non-ASCII Unicode, TOML bare keys are ASCII-only

**File:** `src/init.rs:8-13`

**Issue:** `c.is_alphanumeric()` matches Unicode alphanumeric code points (e.g., `é`, `ñ`, `Ü`, `中`). The TOML specification (v1.0) restricts bare keys to `[A-Za-z0-9_-]` only. A server named `café` or `数据` would produce a bare key that many TOML parsers (including the `toml` crate) reject or handle inconsistently. The doc comment on the function correctly states the intent (`A-Za-z0-9`, `-`, `_`) but the implementation does not enforce it.

**Fix:**

```rust
fn is_valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_'))
}
```

Add a test for non-ASCII input:

```rust
#[test]
fn rejects_unicode_server_names() {
    assert!(!is_valid_server_name("café"));
    assert!(!is_valid_server_name("数据"));
}
```

---

## Warnings

### WR-01: Mutex held across 2-second sleep blocks web server access to handles

**File:** `src/main.rs:482-483, 568-574`

**Issue:** In `run_foreground_loop_shared`, the `Mutex` guard `h` is acquired at line 482 (`let h = handles.lock().await`) and then passed as `&h` to `handle_stdin_command`. Inside `handle_stdin_command`, the `restart` branch sleeps for 2 seconds at line 572 (`tokio::time::sleep(Duration::from_secs(2)).await`) before releasing the guard (the guard is released when `handle_stdin_command` returns and `h` is dropped). During those 2 seconds the web server cannot acquire the same mutex to serve status or log requests — it will hang until the sleep completes.

**Fix:** Drop the lock before sleeping, then re-acquire it to collect the updated state:

```rust
Commands::restart => {
    // Release lock before sleeping.
    drop(h);
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Re-acquire to read updated state.
    let h = handles.lock().await;
    let states = output::collect_states_from_handles(&h);
    output::print_status_table(&states, color);
}
```

Restructure `handle_stdin_command` to take `Arc<Mutex<...>>` instead of `&[ServerHandle]` so it can manage the lock lifetime correctly, or pass a closure/callback for the post-sleep status print.

---

### WR-02: TOCTOU: file re-read after append handle is already open

**File:** `src/init.rs:94-107`

**Issue:** `write_server_entry_to` opens the file in append mode first (lines 94-97), then re-reads the entire file via `std::fs::read_to_string(path)` (line 100-102) to check for a trailing newline. Two file handles are open simultaneously against the same path. The trailing-newline check reads via a second independent `File` open while the append handle is already live. On most OS implementations this is safe, but the ordering is logically inverted — the trailing newline check should happen before opening for append.

**Fix:** Read the existing file first, check for trailing newline, then open for append:

```rust
pub fn write_server_entry_to(path: &Path, toml_block: &str) -> anyhow::Result<()> {
    if path.exists() {
        let existing = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open {} for appending", path.display()))?;

        if !existing.ends_with('\n') {
            file.write_all(b"\n")
                .with_context(|| format!("Failed to write newline to {}", path.display()))?;
        }

        file.write_all(toml_block.as_bytes())
            .with_context(|| format!("Failed to write to {}", path.display()))?;
    } else {
        // ... (unchanged)
    }
    Ok(())
}
```

---

### WR-03: `Arc::try_unwrap` can fail and leave servers without cleanup

**File:** `src/main.rs:194-199, 242-246`

**Issue:** After the daemon's shutdown sequence, `Arc::try_unwrap(handles_arc)` is called to recover ownership of the handles vector for `stop_all_servers`. If any other task still holds a clone of the Arc (e.g., a background task spawned from a control socket handler that hasn't fully returned yet), `try_unwrap` returns `Err` and the function propagates an error — skipping `stop_all_servers`. The child MCP server processes would then be orphaned (running without supervision). The comment notes this risk for the web task and `abort`s it, but the control socket `socket_task` is only `.await`ed with `.ok()` at line 186, which awaits the task JoinHandle but does not guarantee all in-flight requests inside `run_control_socket` have released their `Arc::clone(&daemon_state)`.

**Fix:** Instead of `try_unwrap`, use `Arc::clone` to retrieve the Mutex and lock it:

```rust
// No try_unwrap needed — just lock through the existing Arc.
let mut locked = daemon_state.handles.lock().await;
let final_handles = std::mem::take(&mut *locked);
drop(locked);
supervisor::stop_all_servers(final_handles).await;
```

This avoids the `try_unwrap` failure path entirely and always reaches `stop_all_servers`.

---

### WR-04: `--format` for `gen-config` is a free-form string with no clap-level validation

**File:** `src/cli.rs:67-69`

**Issue:** `GenConfigArgs.format` is `String`. Unknown values (e.g., `--format xml`) are silently accepted by clap and only fail at the `match args.format.as_str()` in `main.rs:393`. Users get no shell-completion hint and the error message only appears at runtime after config loading. This violates the project constraint that public APIs be well-documented and fail fast.

**Fix:** Use a `ValueEnum` so clap validates and documents the accepted values:

```rust
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ConfigFormat {
    Claude,
    Cursor,
}

pub struct GenConfigArgs {
    #[arg(long, short = 'f', value_name = "FORMAT")]
    pub format: ConfigFormat,
    // ...
}
```

Update `main.rs` to match on `ConfigFormat::Claude` and `ConfigFormat::Cursor` instead of string literals.

---

## Info

### IN-01: `--follow` flag on `logs` subcommand is silently ignored

**File:** `src/cli.rs:86-88`, `src/main.rs:320-348`

**Issue:** `LogsArgs.follow` is declared and shown in `--help` output, but the `Commands::Logs` handler in `main.rs` never reads `args.follow`. The field is fully ignored regardless of whether the user passes `-f`. Users expecting live log streaming will see the same snapshot output with or without the flag, with no indication that the feature is unimplemented.

**Suggestion:** Until `--follow` is implemented, either remove the flag or emit an explicit error when it is used:

```rust
Commands::Logs(args) => {
    if args.follow {
        anyhow::bail!("`--follow` is not yet implemented. Follow mode requires daemon mode (planned for a future phase).");
    }
    // ... rest of handler
}
```

---

### IN-02: Inconsistent help-text bullet characters in `handle_stdin_command`

**File:** `src/main.rs:619-626`

**Issue:** The `help` output mixes `—` (em dash) and `-` (hyphen) as separators across different command descriptions within the same block. This is a minor style inconsistency in user-facing output.

**Suggestion:** Standardise on em dash for all entries (matching the majority of existing lines):

```rust
eprintln!("  logs            — Show recent logs from all servers");
eprintln!("  logs <name>     — Show recent logs for a specific server");
```

---

_Reviewed: 2026-04-08_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
