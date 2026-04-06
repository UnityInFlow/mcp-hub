---
phase: 4
slug: web-ui
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-06
---

# Phase 4 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust built-in) + assert_cmd for integration |
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

| Task ID | Plan | Wave | Requirement | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|-----------|-------------------|-------------|--------|
| 04-01-01 | 01 | 1 | WEB-01 | integration | `cargo test web_server_starts` | ❌ W0 | ⬜ pending |
| 04-01-02 | 01 | 1 | WEB-05 | integration | `cargo test health_endpoint` | ❌ W0 | ⬜ pending |
| 04-02-01 | 02 | 1 | WEB-02 | integration | `cargo test status_page` | ❌ W0 | ⬜ pending |
| 04-02-02 | 02 | 1 | WEB-03 | integration | `cargo test tools_page` | ❌ W0 | ⬜ pending |
| 04-03-01 | 03 | 2 | WEB-04 | integration | `cargo test sse_log_stream` | ❌ W0 | ⬜ pending |
| 04-03-02 | 03 | 2 | WEB-04 | integration | `cargo test sse_server_filter` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `tests/web_ui.rs` — integration test stubs for WEB-01 through WEB-05
- [ ] Add `axum`, `askama_web`, `tower-http`, `tokio-stream` to Cargo.toml dev/runtime deps
- [ ] Embed `htmx.min.js` and `htmx-sse.js` as static assets

*Existing test infrastructure (assert_cmd, tempfile, portpicker) covers test framework needs.*

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

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 15s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
