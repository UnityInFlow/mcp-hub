---
phase: 7
reviewers: [codex]
reviewed_at: 2026-04-09T10:00:00Z
plans_reviewed: [07-01-PLAN.md, 07-02-PLAN.md]
---

# Cross-AI Plan Review — Phase 7

## Codex Review (GPT-5.4)

### Plan 07-01

**Summary**
Good prerequisite plan for crates.io readiness and one known cross-compilation issue. It is tightly scoped and mostly appropriate for Wave 1, but it overclaims DIST-01 readiness because the codebase still has other Unix-only IPC assumptions beyond the `nix` dependency.

**Strengths**
- Correctly targets Cargo.toml where `nix` is currently unconditional
- Adds the missing public package artifacts: README, LICENSE, and `readme = "README.md"`
- Uses `cargo publish --dry-run`, which is the right crates.io readiness gate
- Keeps the work small and sequenced before the release workflow

**Concerns**
- **HIGH:** Windows cross-compilation will likely still fail after gating `nix`, because `src/control.rs` uses Unix socket types (`UnixListener`, `UnixStream`) unconditionally
- **MEDIUM:** `cargo check` on the host does not validate DIST-01. Need `cargo check --target x86_64-pc-windows-gnu`
- **MEDIUM:** The crate package may include tracked `.planning` files (~1.1 MB). Add `include` or `exclude` in Cargo.toml
- **MEDIUM:** Verification commands pipe through `tail` without `set -o pipefail`
- **LOW:** README command coverage omits `reload`

**Suggestions**
- Add a task to make daemon/control IPC compile on Windows via `#[cfg(unix)]` stubs
- Add `cargo check --target x86_64-pc-windows-gnu` to Plan 01 acceptance
- Add `include = [...]` to Cargo.toml to exclude `.planning/` from crate

**Risk Assessment:** MEDIUM-HIGH

---

### Plan 07-02

**Summary**
The workflow structure is broadly right: tag trigger, test gate, parallel builds, release aggregation, Homebrew tap update, and crates.io publish. The main risks are release correctness and failure handling.

**Strengths**
- Correct high-level job ordering: test before build, build before release
- Avoids `ubuntu-latest`, matching project runner constraints
- Uses current standard actions: `checkout@v4`, artifact v4, `softprops/action-gh-release@v2`
- Includes human setup gates for secrets, tap repo, crate ownership, and Mac runner

**Concerns**
- **HIGH:** `continue-on-error: true` does not solve offline self-hosted Mac runner — job sits queued until timeout
- **HIGH:** Release job can publish even if Linux ARM64 or macOS failed (only requires linux-x64 success)
- **HIGH:** No CI smoke test proving binaries run (`mcp-hub --version` + `mcp-hub start`)
- **HIGH:** Windows only cross-built, not executed — needs Wine or Windows runner for SC-4
- **MEDIUM:** No tag/package version check — `v0.1.0` could publish a crate still at `0.0.1`
- **MEDIUM:** `cargo install cross --locked` not version-pinned (research chose 0.2.5)
- **LOW:** Homebrew tap update not idempotent for re-runs

**Suggestions**
- Add preflight job checking runner availability, secrets, version/tag match
- Validate all 4 expected artifacts before creating GitHub Release
- Add smoke tests in build jobs before packaging
- Pin `cross` version
- Run `cargo publish --dry-run` before GitHub Release, actual publish after
- Generate SHA256SUMS file for all archives

**Risk Assessment:** HIGH

---

## Consensus Summary

### Agreed Strengths
- Well-decomposed into "crate readiness" (Wave 1) and "release automation" (Wave 2)
- Correct use of standard GitHub Actions patterns
- Good human checkpoint for infrastructure verification

### Agreed Concerns
1. **Windows compilation beyond `nix`** (HIGH) — `src/control.rs` Unix sockets are unconditional. Plan 01 nix fix is insufficient for Windows.
2. **No binary smoke tests** (HIGH) — SC-4 requires `mcp-hub --version` + `mcp-hub start` on all platform binaries.
3. **Partial release risk** (HIGH) — Release can publish with missing platform artifacts.
4. **Mac runner offline handling** (HIGH) — `continue-on-error` doesn't solve queued-forever jobs.
5. **Crate packaging includes .planning/** (MEDIUM) — Bloats the crate unnecessarily.

### Divergent Views
None — single reviewer. Would benefit from Gemini review for second opinion.
