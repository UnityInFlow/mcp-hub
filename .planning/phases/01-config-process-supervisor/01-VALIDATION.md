---
phase: 1
slug: config-process-supervisor
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-03
---

# Phase 1 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in `#[cfg(test)]` + `tokio::test` + `assert_cmd` (integration) |
| **Config file** | `Cargo.toml` (dev-dependencies section) |
| **Quick run command** | `cargo test --lib` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~10 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd:verify-work`:** Full suite must be green
- **Max feedback latency:** 15 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|-----------|-------------------|-------------|--------|
| 01-01 | 01 | 1 | CFG-01,CFG-02 | unit | `cargo test config::` | ❌ W0 | ⬜ pending |
| 01-02 | 02 | 1 | PROC-01,PROC-08,PROC-09 | unit | `cargo test supervisor::` | ❌ W0 | ⬜ pending |
| 01-03 | 02 | 1 | PROC-05,PROC-06 | unit | `cargo test supervisor::backoff` | ❌ W0 | ⬜ pending |
| 01-04 | 02 | 1 | PROC-07,DMN-01 | unit | `cargo test supervisor::shutdown` | ❌ W0 | ⬜ pending |
| 01-05 | 03 | 2 | PROC-02,PROC-03 | integration | `cargo test --test cli_` | ❌ W0 | ⬜ pending |
| 01-06 | 03 | 2 | CFG-01,CFG-02 | integration | `cargo test --test config_` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending / ✅ green / ❌ red / ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `Cargo.toml` — dev-dependencies: assert_cmd, predicates, tempfile, portpicker, tokio-test
- [ ] `tests/` directory — created for integration tests
- [ ] `src/config.rs` — module stub for unit test compilation
- [ ] `src/supervisor.rs` — module stub for unit test compilation

*Existing infrastructure: None — greenfield project, Wave 0 must scaffold everything.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Ctrl+C graceful shutdown | PROC-07 | Signal handling requires terminal interaction | Start hub with 2+ servers, press Ctrl+C, verify all children exit within 5s |
| No zombie processes after stop | PROC-09 | Requires `ps` inspection | Start/stop hub 10 times, check `ps aux` for orphans |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 15s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
