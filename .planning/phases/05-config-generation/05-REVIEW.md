---
phase: 05-config-generation
reviewed: 2026-04-08T00:00:00Z
depth: standard
files_reviewed: 7
files_reviewed_list:
  - Cargo.toml
  - src/cli.rs
  - src/control.rs
  - src/gen_config.rs
  - src/lib.rs
  - src/main.rs
  - tests/gen_config_test.rs
findings:
  critical: 0
  warning: 4
  info: 4
  total: 8
status: issues_found
---

# Phase 5: Code Review Report

**Reviewed:** 2026-04-08T00:00:00Z
**Depth:** standard
**Files Reviewed:** 7
**Status:** issues_found

## Summary

This phase adds the `gen-config` subcommand (`src/gen_config.rs`), its CLI surface (`src/cli.rs`), integration tests (`tests/gen_config_test.rs`), and connects everything through `src/main.rs`. The config generation logic itself is clean and well-structured: alphabetical key ordering, HTTP transport exclusion with warnings, and live comment injection are all correct. The unit tests in `gen_config.rs` and the integration tests in `tests/gen_config_test.rs` are thorough.

Two warnings involve a real crash path in `src/main.rs` — the foreground mode `Arc::try_unwrap` will reliably fail at runtime because the web task holds a clone of the `Arc` and is never aborted before the unwrap. A second warning covers the daemon mode shutdown path, which has the same structural risk. Two additional warnings cover blocking I/O on the async thread in `control.rs` and a silently-swallowed empty string in `gen_config.rs`.

---

## Warnings

### WR-01: `Arc::try_unwrap` will always fail in foreground mode — servers not stopped on exit

**File:** `src/main.rs:218-241`

**Issue:** In foreground mode the web server task is spawned with its handle discarded into `_web_task` (the `_` prefix means the value is dropped immediately). The task holds `Arc::clone(&handles_arc)`. When `run_foreground_loop_shared` returns, the code immediately calls `Arc::try_unwrap(handles_arc)`. Because the detached web task still owns one reference count, `try_unwrap` returns `Err`, hits the `map_err` bail, and the process exits with an error — without ever calling `supervisor::stop_all_servers`. Child MCP server processes are left orphaned.

```rust
// Current (broken):
let _web_task = tokio::spawn(async move {
    if let Err(e) =
        web::start_web_server(config.hub.web_port, web_state, web_shutdown).await
    { ... }
});
// ...
let final_handles = Arc::try_unwrap(handles_arc)   // <-- always Err
    .map_err(|_| anyhow::anyhow!("Cannot unwrap foreground handles — Arc still shared"))?
    .into_inner();
supervisor::stop_all_servers(final_handles).await;
```

**Fix:** Abort the web task and await it before attempting `try_unwrap`, or use a `Mutex` instead of depending on unique ownership:

```rust
// Option A: abort and await
let web_task = tokio::spawn(async move {
    if let Err(e) =
        web::start_web_server(config.hub.web_port, web_state, web_shutdown).await
    { tracing::error!("Web UI error: {e}"); }
});

run_foreground_loop_shared(&handles_arc, color, Arc::clone(&log_agg)).await?;

tracing::info!("Shutting down all servers...");
shutdown.cancel();

web_task.abort();
let _ = web_task.await;  // drain to ensure Arc refcount is released

let final_handles = Arc::try_unwrap(handles_arc)
    .map_err(|_| anyhow::anyhow!("Cannot unwrap foreground handles — Arc still shared"))?
    .into_inner();
supervisor::stop_all_servers(final_handles).await;
```

---

### WR-02: Race between `web_task.abort()` and `Arc::try_unwrap` in daemon mode

**File:** `src/main.rs:186-197`

**Issue:** In daemon mode, `web_task.abort()` is called and then immediately `Arc::try_unwrap(handles_arc)` is attempted. `abort()` schedules cancellation but does not wait for the task to actually complete and release its `Arc` clone. If the task is mid-execution when `abort()` is called, the `Arc` refcount is still > 1 at the point of `try_unwrap`, causing the same bail-without-cleanup failure as WR-01.

```rust
web_task.abort(); // async cancel — not yet complete
// ...
let final_handles = Arc::try_unwrap(handles_arc)  // <-- may be Err
    .map_err(|_| anyhow::anyhow!("Cannot unwrap daemon handles — Arc still shared"))?
    .into_inner();
supervisor::stop_all_servers(final_handles).await;  // <-- may never reach here
```

**Fix:** Await the aborted task to ensure it has fully exited before the `try_unwrap`:

```rust
web_task.abort();
let _ = web_task.await;  // wait for task to release its Arc clone

let final_handles = Arc::try_unwrap(handles_arc)
    .map_err(|_| anyhow::anyhow!("Cannot unwrap daemon handles — Arc still shared"))?
    .into_inner();
supervisor::stop_all_servers(final_handles).await;
```

---

### WR-03: Blocking filesystem I/O on the async thread in `control.rs`

**File:** `src/control.rs:119` and `src/control.rs:151`

**Issue:** `std::fs::remove_file(sock_path)` is a blocking syscall called directly from async context inside `run_control_socket`. Tokio's executor runs on a thread pool where blocking calls stall a worker thread, potentially causing task starvation under load. Both socket cleanup sites (pre-bind cleanup on line 119 and post-shutdown cleanup on line 151) are affected.

```rust
// Line 119 — before binding
let _ = std::fs::remove_file(sock_path);

// Line 151 — after shutdown
let _ = std::fs::remove_file(sock_path);
```

**Fix:** Use `tokio::fs::remove_file` (async) or `tokio::task::spawn_blocking` for file removal. Since these are single operations called infrequently, `tokio::fs` is the simplest fix:

```rust
// Pre-bind cleanup
let _ = tokio::fs::remove_file(sock_path).await;

// Post-shutdown cleanup (the function is already async)
let _ = tokio::fs::remove_file(sock_path).await;
```

---

### WR-04: Silent empty-string fallback for required `name` field in `parse_live_info`

**File:** `src/gen_config.rs:91`

**Issue:** When parsing a daemon Status response entry, the `name` field uses `unwrap_or_default()` which produces an empty string if the field is absent or not a string. A server with an empty name will never match any config key in `inject_live_comments` (line 219), so live annotations are silently dropped. If a future daemon response format change omits `name`, users get no error — they just get output with no live comments, which is hard to diagnose.

```rust
// Current:
name: s["name"].as_str().unwrap_or_default().to_string(),
```

**Fix:** Return an error for a missing or invalid name, consistent with how the rest of the function uses `anyhow::ensure` and the `?` operator:

```rust
name: s["name"]
    .as_str()
    .filter(|n| !n.is_empty())
    .context("Server entry missing 'name' field in status response")?
    .to_string(),
```

---

## Info

### IN-01: `--format` flag should use a `ValueEnum` for compile-time validation

**File:** `src/cli.rs:65`

**Issue:** `pub format: String` accepts any string and defers validation to a `match` block at runtime in `main.rs:382`. Using a clap `ValueEnum` would surface invalid values at argument parsing, auto-generate `--help` with valid options, and remove the need for the manual `other => bail!(...)` arm.

**Fix:**
```rust
// In cli.rs
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ConfigFormat {
    Claude,
    Cursor,
}

// In GenConfigArgs
pub format: ConfigFormat,

// In main.rs the match becomes exhaustive:
match args.format {
    ConfigFormat::Claude => gen_config::render_claude_config(&config, live_info.as_deref())?,
    ConfigFormat::Cursor => gen_config::render_cursor_config(&config, live_info.as_deref())?,
}
```

---

### IN-02: Dead fields `resource_count` and `prompt_count` in `ServerLiveInfo`

**File:** `src/gen_config.rs:27-31`

**Issue:** `resource_count` and `prompt_count` are parsed from the daemon response and stored in `ServerLiveInfo`, but are immediately suppressed with `#[allow(dead_code)]`. They are never read anywhere. Parsing unused data increases code surface without benefit.

**Fix:** Either remove the fields and their parsing, or add resource/prompt count annotations to the `inject_live_comments` output so the fields are actually used. Removing is simpler until there is a concrete use case:

```rust
pub struct ServerLiveInfo {
    pub name: String,
    pub state: String,
    pub tool_names: Vec<String>,
    // resource_count and prompt_count removed until used
}
```

---

### IN-03: Fake token in test fixture is committed to source

**File:** `tests/gen_config_test.rs:37`

**Issue:** The `two_servers_toml()` fixture contains `GITHUB_TOKEN = "ghp_test123"`. This matches the token patterns that `S003-no-secrets` from the sibling `spec-linter` tool would flag. While this is clearly a fake test credential, it sets a precedent of committing token-shaped values and would fail the project's own lint tooling.

**Fix:** Use an environment variable reference that makes the test-only nature explicit and does not resemble a real token format:

```toml
env = { GITHUB_TOKEN = "test-token-placeholder" }
```

And update the integration test assertion from `"ghp_test123"` to `"test-token-placeholder"`.

---

### IN-04: Inconsistent use of `—` vs `-` in help text

**File:** `src/main.rs:611-617`

**Issue:** The `help` command output mixes em-dash (`—`) and regular hyphen (`-`) as separators within the same block, making the CLI help text visually inconsistent.

```rust
eprintln!("  restart <name>  — Restart the named server");  // em-dash
eprintln!("  logs            - Show recent logs from all servers");  // hyphen
eprintln!("  logs <name>     - Show recent logs for a specific server");  // hyphen
```

**Fix:** Standardise to em-dash throughout:
```rust
eprintln!("  logs            — Show recent logs from all servers");
eprintln!("  logs <name>     — Show recent logs for a specific server");
```

---

_Reviewed: 2026-04-08T00:00:00Z_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
