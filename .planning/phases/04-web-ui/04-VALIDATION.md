---
phase: 4
slug: web-ui
status: draft
nyquist_compliant: true
wave_0_complete: false
created: 2026-04-06
updated: 2026-04-06
---

# Phase 4 -- Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust built-in) + tower::ServiceExt for in-module route tests |
| **Config file** | `Cargo.toml` (already configured) |
| **Quick run command** | `cargo test --lib` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~15 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd:verify-work`:** Full suite must be green
- **Max feedback latency:** 15 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Test Type | Automated Command | Status |
|---------|------|------|-------------|-----------|-------------------|--------|
| 04-01-01 | 01 | 1 | WEB-01 | unit | `cargo test --lib -- config::tests::hub_config_web_port config::tests::hub_config_default_port` | pending |
| 04-01-02 | 01 | 1 | WEB-01 | build | `cargo build` (compiles web module skeleton) | pending |
| 04-02-01 | 02 | 2 | WEB-02 | build | `cargo build` (askama compile-time template check) | pending |
| 04-02-02 | 02 | 2 | WEB-04 | build | `cargo build` (SSE handler compiles) | pending |
| 04-03-01 | 03 | 3 | WEB-01..05 | unit | `cargo test --lib -- web::routes::tests` | pending |
| 04-03-02 | 03 | 3 | ALL | manual | Human browser verification checkpoint | pending |

*Status: pending / green / red / flaky*

---

## Wave 0 Requirements

- [x] All auto tasks have `<automated>` verify commands (Nyquist compliant)
- [ ] `tower` and `http-body-util` added to [dev-dependencies] in Cargo.toml (done in Plan 03 Task 1)
- [ ] In-module test helpers (`make_test_state`) created (done in Plan 03 Task 1)

*Existing test infrastructure (assert_cmd, tempfile, portpicker) covers external test needs.*
*In-module tests using tower::ServiceExt::oneshot avoid the binary crate limitation.*

---

## Automated Test Coverage

All auto tasks have `<automated>` verify commands:

| Plan | Task | Automated Verify |
|------|------|-----------------|
| 01 | Task 1 | `cargo test --lib -- config::tests::hub_config_web_port config::tests::hub_config_default_port` |
| 01 | Task 2 | `cargo build && cargo clippy -- -D warnings` |
| 02 | Task 1 | `cargo build && cargo clippy -- -D warnings` |
| 02 | Task 2 | `cargo build && cargo clippy -- -D warnings` |
| 03 | Task 1 | `cargo test --lib -- web::routes::tests` |
| 03 | Task 2 | Human checkpoint (not automated -- visual verification) |

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Card grid renders correctly in browser | WEB-02 | Visual layout verification | Open http://localhost:3456, verify card grid with server info |
| HTMX auto-refresh updates cards | WEB-02 | Requires browser with JS | Open status page, change server state, observe card update within 5s |
| SSE log streaming with filter pills | WEB-04 | Requires browser SSE support | Open logs page, click server pills, verify filtered stream |
| Tab bar navigation with HTMX | WEB-01 | Requires browser with JS | Click Status/Tools/Logs tabs, verify content swap |

---

## Validation Sign-Off

- [x] All tasks have `<automated>` verify or are checkpoint:human-verify
- [x] Sampling continuity: no 3 consecutive tasks without automated verify
- [x] No watch-mode flags
- [x] Feedback latency < 15s
- [x] `nyquist_compliant: true` set in frontmatter

**Approval:** pending execution
