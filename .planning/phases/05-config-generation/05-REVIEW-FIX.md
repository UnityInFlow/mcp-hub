---
phase: 05-config-generation
fixed_at: 2026-04-08T20:21:41Z
review_path: .planning/phases/05-config-generation/05-REVIEW.md
iteration: 1
findings_in_scope: 4
fixed: 4
skipped: 0
status: all_fixed
---

# Phase 5: Code Review Fix Report

**Fixed at:** 2026-04-08T20:21:41Z
**Source review:** .planning/phases/05-config-generation/05-REVIEW.md
**Iteration:** 1

**Summary:**
- Findings in scope: 4 (WR-01 through WR-04)
- Fixed: 4
- Skipped: 0

## Fixed Issues

### WR-01: `Arc::try_unwrap` will always fail in foreground mode — servers not stopped on exit

**Files modified:** `src/main.rs`
**Commit:** 58289d3
**Applied fix:** Changed `let _web_task = tokio::spawn(...)` to `let web_task = tokio::spawn(...)` (removing the `_` prefix so the handle is kept). After `run_foreground_loop_shared` returns and `shutdown.cancel()` is called, added `web_task.abort(); let _ = web_task.await;` before the `Arc::try_unwrap` call. This ensures the spawned task fully exits and releases its `Arc` clone, so `try_unwrap` succeeds and `stop_all_servers` is always reached.

---

### WR-02: Race between `web_task.abort()` and `Arc::try_unwrap` in daemon mode

**Files modified:** `src/main.rs`
**Commit:** 58289d3 (committed together with WR-01 — both changes are in src/main.rs)
**Applied fix:** Added `let _ = web_task.await;` immediately after `web_task.abort()` in the daemon mode shutdown path. This waits for the task to fully exit before attempting `Arc::try_unwrap`, eliminating the race where the task's `Arc` clone could still be live at the point of unwrap.

---

### WR-03: Blocking filesystem I/O on the async thread in `control.rs`

**Files modified:** `src/control.rs`
**Commit:** f443acd
**Applied fix:** Replaced both `std::fs::remove_file(sock_path)` calls with `tokio::fs::remove_file(sock_path).await` — the pre-bind cleanup (line 119) and the post-shutdown cleanup (line 151). Since `run_control_socket` is already an `async fn`, the `.await` integrates cleanly with no signature changes.

---

### WR-04: Silent empty-string fallback for required `name` field in `parse_live_info`

**Files modified:** `src/gen_config.rs`
**Commit:** ac3f263
**Applied fix:** Replaced `s["name"].as_str().unwrap_or_default().to_string()` with a chain that uses `.filter(|n| !n.is_empty()).context("Server entry missing 'name' field in status response")?`. This propagates an error when `name` is absent or empty, consistent with the surrounding `anyhow::ensure` and `?` usage in the same function.

---

_Fixed: 2026-04-08T20:21:41Z_
_Fixer: Claude (gsd-code-fixer)_
_Iteration: 1_
