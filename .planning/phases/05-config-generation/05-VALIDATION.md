---
phase: 5
slug: config-generation
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-08
---

# Phase 5 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in `#[test]` + `assert_cmd` + `predicates` |
| **Config file** | `Cargo.toml` (already configured) |
| **Quick run command** | `cargo test --lib gen_config` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~15 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib gen_config`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 15 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 05-01-01 | 01 | 1 | GEN-01 | — | N/A | unit | `cargo test --lib gen_config::tests::claude_format` | ❌ W0 | ⬜ pending |
| 05-01-02 | 01 | 1 | GEN-02 | — | N/A | unit | `cargo test --lib gen_config::tests::cursor_format` | ❌ W0 | ⬜ pending |
| 05-01-03 | 01 | 1 | GEN-03 | — | N/A | unit | `cargo test --lib gen_config::tests::offline_mode` | ❌ W0 | ⬜ pending |
| 05-01-04 | 01 | 1 | GEN-05 | — | N/A | unit | `cargo test --lib gen_config::tests::version_comment` | ❌ W0 | ⬜ pending |
| 05-01-05 | 01 | 1 | GEN-05 | — | N/A | unit | `cargo test --lib gen_config::tests::zero_servers` | ❌ W0 | ⬜ pending |
| 05-02-01 | 02 | 1 | GEN-04 | — | N/A | integration | `cargo test --test gen_config_live` | ❌ W0 | ⬜ pending |
| 05-02-02 | 02 | 1 | GEN-01 | — | N/A | integration | `cargo test --test gen_config_cli` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `src/gen_config.rs` — module with unit test stubs
- [ ] Test fixtures (TOML configs with various server configurations)

*Existing test infrastructure (assert_cmd, predicates, tempfile) covers integration test needs.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| `--live` with actual daemon | GEN-04 | Requires running MCP servers | Start hub with test config, run `mcp-hub gen-config --format claude --live`, verify tool names in comments |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 15s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
