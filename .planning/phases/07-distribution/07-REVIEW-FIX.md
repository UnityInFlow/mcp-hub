---
phase: 07-distribution
fixed_at: 2026-04-08T00:00:00Z
review_path: .planning/phases/07-distribution/07-REVIEW.md
iteration: 1
findings_in_scope: 4
fixed: 4
skipped: 0
status: all_fixed
---

# Phase 7: Code Review Fix Report

**Fixed at:** 2026-04-08
**Source review:** .planning/phases/07-distribution/07-REVIEW.md
**Iteration:** 1

**Summary:**
- Findings in scope: 4 (CR-01, WR-01, WR-02, WR-03; IN-* excluded by fix_scope)
- Fixed: 4
- Skipped: 0

## Fixed Issues

### CR-01: HOMEBREW_TAP_TOKEN leaked via git remote URL

**Files modified:** `.github/workflows/release.yml`
**Commit:** e3d685d
**Applied fix:** Replaced `git clone "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/..."` with a plain HTTPS clone followed by a git credential helper configuration. The token is now supplied via a shell function credential helper (`git config credential.helper '!f() { ...; }; f'`) so it never appears in the remote URL, git error messages, or remote-v output.

---

### WR-01: Homebrew formula written with leading whitespace — malformed Ruby file

**Files modified:** `.github/workflows/release.yml`
**Commit:** e3d685d
**Applied fix:** Rewrote the heredoc so that the `cat >` line and the `FORMULA` delimiter are at column 0 within the shell script (valid inside YAML `run: |` scalars). The Ruby class body is now unindented — every line of `mcp-hub.rb` begins at column 0, matching what `brew style` expects. Previously all lines had 10 spaces of leading whitespace from YAML indentation.

---

### WR-02: `cd tap-repo` not guarded — silent wrong-directory execution if clone fails

**Files modified:** `.github/workflows/release.yml`
**Commit:** e3d685d
**Applied fix:** Changed `cd tap-repo` to `cd tap-repo || { echo "ERROR: tap-repo directory not found after clone"; exit 1; }`. The restructured step also eliminates the later bare `cd tap-repo` (which followed the heredoc) by keeping all git operations inside `tap-repo` after the single guarded `cd`.

---

### WR-03: `cargo test` and `cargo publish` run without `--locked`

**Files modified:** `.github/workflows/release.yml`
**Commit:** 2061195
**Applied fix:** Added `--locked` to three cargo commands:
- Line 50: `cargo publish --dry-run --locked` (preflight job)
- Line 68: `cargo test --locked` (test job)
- Line 411: `cargo publish --locked` (release job)

This ensures Cargo.lock is respected throughout the release pipeline, so the tested and released binary use identical transitive dependency versions.

---

_Fixed: 2026-04-08_
_Fixer: Claude (gsd-code-fixer)_
_Iteration: 1_
