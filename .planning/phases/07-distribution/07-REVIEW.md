---
phase: 07-distribution
reviewed: 2026-04-08T00:00:00Z
depth: standard
files_reviewed: 5
files_reviewed_list:
  - Cargo.toml
  - src/control.rs
  - .github/workflows/release.yml
  - README.md
  - LICENSE
findings:
  critical: 1
  warning: 3
  info: 2
  total: 6
status: issues_found
---

# Phase 7: Code Review Report

**Reviewed:** 2026-04-08
**Depth:** standard
**Files Reviewed:** 5
**Status:** issues_found

## Summary

Five files covering the distribution layer were reviewed: the Cargo manifest, the Unix socket IPC module, the GitHub Actions release workflow, README, and LICENSE.

`src/control.rs` is well-written. It follows project conventions consistently — no `unwrap()`, proper `?` propagation, all public items carry `///` doc comments, exhaustive match on `DaemonRequest` variants, and correct `usize` arithmetic (the `all.len() > lines` guard prevents underflow on line 256). No findings in this file.

The main concerns are in the release workflow. One critical security issue (`HOMEBREW_TAP_TOKEN` exposed in a git remote URL), two workflow correctness warnings (Homebrew formula indentation producing malformed Ruby output, and `cd` without `|| exit` after a potentially failing `git clone`), and a missed `--locked` flag on cargo commands. `Cargo.toml` and `README.md` are clean aside from two informational items.

---

## Critical Issues

### CR-01: HOMEBREW_TAP_TOKEN leaked via git remote URL

**File:** `.github/workflows/release.yml:371`
**Issue:** The token is interpolated directly into the `git clone` URL:
```
git clone "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/..."
```
Even though GitHub Actions masks the secret value in log output, the full URL (including the credential) can appear in Git error messages, `git remote -v` output, and third-party action logs that process the runner's stdout before masking takes effect. The standard safe pattern is to configure the credential helper so the token never appears in the URL.

**Fix:**
```yaml
- name: Update Homebrew tap
  env:
    HOMEBREW_TAP_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
  run: |
    TAG="${GITHUB_REF##refs/tags/}"
    VERSION="${TAG#v}"
    SHA256=$(sha256sum artifacts/mcp-hub-macos-universal.tar.gz | awk '{print $1}')

    git clone https://github.com/unityinflow/homebrew-tap.git tap-repo
    cd tap-repo || exit 1

    git config user.name "github-actions[bot]"
    git config user.email "github-actions[bot]@users.noreply.github.com"

    # Use credential helper -- token never appears in remote URL
    git config credential.helper '!f() { echo "username=x-access-token"; echo "password=${HOMEBREW_TAP_TOKEN}"; }; f'

    # ... write formula, then:
    git add Formula/mcp-hub.rb
    git commit -m "chore: bump mcp-hub to ${VERSION}"
    git push
```

---

## Warnings

### WR-01: Homebrew formula written with leading whitespace — malformed Ruby file

**File:** `.github/workflows/release.yml:374-394`
**Issue:** The `cat > ... << FORMULA` heredoc body is indented 10 spaces to align with the surrounding YAML `run:` block. Because the heredoc delimiter `FORMULA` is unquoted (not `<< 'FORMULA'`), shell variable substitution occurs (which is intentional), but the leading whitespace is **not** stripped — Bash only strips leading tabs with `<<-`, not spaces. The resulting `mcp-hub.rb` file will have every line prefixed with 10 spaces, producing a Ruby class that begins with `          class McpHub < Formula`. While Ruby/Homebrew's formula parser tolerates this at present, it is non-standard, will fail `brew style` linting, and may break on future Homebrew versions that enforce stricter indentation.

**Fix:** Use `<<-FORMULA` (strip leading tabs) and switch the body to tab-indented, or write a Python/Ruby one-liner to generate the formula. The simplest approach is to write a `Formula/mcp-hub.rb.template` into the repo and substitute values with `sed` at release time:
```yaml
      - name: Update Homebrew tap
        run: |
          ...
          # No heredoc indentation issue: write at column 0
          cat > tap-repo/Formula/mcp-hub.rb << FORMULA
class McpHub < Formula
  desc "PM2 for MCP servers - manage, monitor, and configure your MCP servers"
  homepage "https://github.com/UnityInFlow/mcp-hub"
  version "${VERSION}"
  license "MIT"

  on_macos do
    url "https://github.com/UnityInFlow/mcp-hub/releases/download/${TAG}/mcp-hub-macos-universal.tar.gz"
    sha256 "${SHA256}"
  end

  def install
    bin.install "mcp-hub"
  end

  test do
    system "#{bin}/mcp-hub", "--version"
  end
end
FORMULA
```
(This requires dedenting the `cat >` line and the delimiter to column 0 within the YAML `run` block — valid YAML since the shell script content is a scalar string.)

---

### WR-02: `cd tap-repo` not guarded — silent wrong-directory execution if clone fails

**File:** `.github/workflows/release.yml:396`
**Issue:** GitHub Actions `run:` blocks execute with `set -e` by default, so a failed `git clone` on line 371 will abort the step before `cd tap-repo` is reached — this is safe in that specific failure path. However, if the clone succeeds but the directory is created with a different name (e.g., due to a future refactor), subsequent `git add` / `git commit` / `git push` commands would execute in the workflow's working directory (the checked-out `mcp-hub` repo), potentially committing and pushing to the wrong repository. Explicit guarding makes the intent clear and prevents silent misbehaviour.

**Fix:**
```bash
cd tap-repo || { echo "ERROR: tap-repo directory not found after clone"; exit 1; }
```

---

### WR-03: `cargo test` and `cargo publish` run without `--locked`

**File:** `.github/workflows/release.yml:68` and `release.yml:407`
**Issue:** `cargo test` in the `test` job and `cargo publish` in the `release` job do not pass `--locked`. Without `--locked`, Cargo may resolve dependency versions that differ from `Cargo.lock`, meaning the binary shipped in a release could use different transitive dependency versions than those tested locally. For a release workflow this is a reproducibility risk.

**Fix:**
```yaml
# test job, line 68
run: cargo test --locked

# release job, line 407
run: cargo publish --locked
```
`cargo publish --dry-run` in the preflight job (line 50) should also use `--locked` for the same reason.

---

## Info

### IN-01: `rust-version` (MSRV) not declared in Cargo.toml

**File:** `Cargo.toml`
**Issue:** No `rust-version` field is set. Without a declared Minimum Supported Rust Version, users on older toolchains get an opaque compile error. The project uses Rust edition 2021 and Tokio 1.x which require at least Rust 1.70.

**Fix:**
```toml
[package]
name = "mcp-server-hub"
version = "0.0.1"
edition = "2021"
rust-version = "1.75"   # or the minimum version verified to compile
```

---

### IN-02: `DaemonState.color` field is permanently dead

**File:** `src/control.rs:102-103`
**Issue:** The `color: bool` field carries `#[allow(dead_code)]` with the comment "Reserved for future use". While the suppression prevents the compiler warning, it leaves a field with no read site. If colorized log output in the daemon is genuinely planned, tracking it as an issue (or removing it and re-adding it when implemented) is cleaner than carrying a suppression permanently.

**Fix:** Remove the field and the `#[allow(dead_code)]` attribute until the feature is implemented. Re-add it with the actual usage in the same commit.

---

_Reviewed: 2026-04-08_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
