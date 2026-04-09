---
phase: 07-distribution
plan: 02
subsystem: distribution
tags: [rust, github-actions, release, homebrew, crates-io, cross-compilation, ci-cd]
requirements: [DIST-01, DIST-02, DIST-03]

dependency_graph:
  requires: [07-01]
  provides: [release-pipeline, homebrew-formula-automation, crates-io-publish]
  affects: []

tech_stack:
  added:
    - softprops/action-gh-release@v2 (GitHub Release creation)
    - cross@0.2.5 (cross-compilation, pinned)
    - actions/upload-artifact@v4 (artifact storage between jobs)
    - actions/download-artifact@v4 (artifact retrieval in release job)
  patterns:
    - Multi-job workflow with explicit `needs` chains (preflight -> test -> build -> release)
    - SC-4 smoke test pattern: inline fixture TOML + `mcp-hub start` with timeout
    - macOS universal binary via `lipo -create`
    - Homebrew formula auto-generation with computed SHA256 from actual artifact
    - All-or-nothing release: `needs: [build-linux-x64, build-linux-arm64, build-macos]`

key_files:
  created:
    - .github/workflows/release.yml
  modified: []

decisions:
  - cross@0.2.5 pinned (not latest) to ensure reproducible builds across runner invocations
  - Release job requires ALL three build jobs (prevents partial releases)
  - macOS job has timeout-minutes:30 (prevents infinite queue when developer Mac is offline)
  - Windows smoke test is best-effort via Wine (accepted limitation -- cross-compilation success validates binary)
  - cargo publish runs last in release job (after binaries uploaded + Homebrew tap updated)
  - Homebrew formula uses `bin.install "mcp-hub"` -- lipo output named `mcp-hub` (not `mcp-hub-universal`)
  - preflight job validates tag/version match AND runs cargo publish --dry-run before any build starts

metrics:
  duration: ~12 minutes
  completed: 2026-04-08
  tasks_completed: 1
  tasks_total: 2
  files_changed: 1
---

# Phase 7 Plan 02: Release Pipeline Summary

Complete GitHub Actions release pipeline that builds binaries for 4 platform variants (Linux x86_64, Linux aarch64, Windows x86_64, macOS universal), runs SC-4 smoke tests on each, creates a GitHub Release, auto-updates the Homebrew tap formula with correct SHA256, and publishes to crates.io -- triggered by a single `git tag v* && git push` command.

## Status: PARTIAL (awaiting Task 2 checkpoint)

Task 1 is complete. Task 2 (human-verify: infrastructure readiness) is pending user confirmation.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create release.yml with test gate, build matrix, smoke tests, and release job | f4c6fad | .github/workflows/release.yml |

## Tasks Pending

| Task | Name | Type | Blocked by |
|------|------|------|------------|
| 2 | Verify release workflow and infrastructure readiness | checkpoint:human-verify | User must confirm: workflow structure, secrets, runners, homebrew-tap repo, crate name availability |

## What Was Built

### Task 1: `.github/workflows/release.yml`

6-job release pipeline:

**Job 1: `preflight`** (runs on `arc-runner-unityinflow`)
- Extracts version from git tag, extracts version from Cargo.toml, fails if they don't match
- Runs `cargo publish --dry-run` to catch packaging issues before any build starts
- Exports `version` output for downstream jobs

**Job 2: `test`** (needs: preflight, runs on `arc-runner-unityinflow`)
- `cargo test` (full test suite, currently 226 tests)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`

**Job 3: `build-linux-x64`** (needs: test, runs on `arc-runner-unityinflow`)
- Installs `cross@0.2.5 --locked` (pinned for reproducibility)
- Builds `x86_64-unknown-linux-gnu` (native via cross)
- Smoke tests Linux x86_64: `--version` + `mcp-hub start -c /tmp/smoke-test.toml` with inline TOML fixture
- Builds `x86_64-pc-windows-gnu` (cross-compiled)
- Windows smoke test via Wine (best-effort -- skipped with clear message if Wine absent)
- Packages: `mcp-hub-linux-x86_64.tar.gz`, `mcp-hub-windows-x86_64.zip`
- SHA256 checksums for both archives

**Job 4: `build-linux-arm64`** (needs: test, runs on `orangepi`)
- Adds `aarch64-unknown-linux-gnu` rustup target
- Builds natively on ARM64 OrangePi runner
- Smoke tests: `--version` + `mcp-hub start -c /tmp/smoke-test.toml` with inline TOML fixture
- Packages: `mcp-hub-linux-aarch64.tar.gz`

**Job 5: `build-macos`** (needs: test, runs on `macos-arm64`, timeout-minutes: 30)
- Adds `x86_64-apple-darwin` target
- Builds both `aarch64-apple-darwin` and `x86_64-apple-darwin`
- Combines via `lipo -create` into universal binary named `mcp-hub`
- Smoke tests universal binary: `--version` + `mcp-hub start -c /tmp/smoke-test.toml`
- Packages: `mcp-hub-macos-universal.tar.gz`
- `timeout-minutes: 30` prevents infinite queue if developer Mac is offline

**Job 6: `release`** (needs: `[build-linux-x64, build-linux-arm64, build-macos]`, runs on `arc-runner-unityinflow`)
- `permissions: contents: write`
- Downloads all artifacts with `merge-multiple: true`
- Validates all 4 archives exist before proceeding (fails with `MISSING: filename` if any absent)
- Generates `SHA256SUMS.txt` via `sha256sum` over all 4 archives
- Creates GitHub Release via `softprops/action-gh-release@v2` with all 5 files + install instructions
- Updates `unityinflow/homebrew-tap` Formula/mcp-hub.rb with computed SHA256 + version from tag
- `cargo publish` (last step, after all other distribution is complete)

### SC-4 Smoke Test Pattern (applied to all 3 build jobs)

Each build job creates an inline fixture config and runs `mcp-hub start` against it:
```toml
[servers.echo-test]
command = "echo"
args = ["hello"]
```
The `echo` server exits immediately; `mcp-hub` loads config, spawns the process, detects exit, and terminates. `timeout 10` prevents hangs. Non-panic exit (even non-zero from server dying) satisfies SC-4. This validates config parsing and server startup logic beyond just `--version`.

## Threat Mitigations Applied

| Threat | Mitigation |
|--------|------------|
| T-07-04: Artifact tampering | SHA256SUMS.txt generated in release job from actual artifacts |
| T-07-05: HOMEBREW_TAP_TOKEN exposure | Stored as GitHub secret, used only in release job |
| T-07-06: Tag/version mismatch | preflight job compares git tag version vs Cargo.toml version |
| T-07-07: Mac runner offline DoS | timeout-minutes: 30 on macOS job; release needs all builds |
| T-07-09: Homebrew formula spoofing | Formula URL from GITHUB_REF tag; SHA256 from actual artifact |

## Deviations from Plan

None -- plan executed exactly as written. All Codex review HIGH concerns addressed:
- SC-4 (start smoke test with fixture config): implemented in all 3 build jobs
- Partial release prevention: `needs: [build-linux-x64, build-linux-arm64, build-macos]`
- Mac runner timeout: `timeout-minutes: 30` on macOS job
- Tag/version consistency: preflight job comparison
- Pinned cross version: `cross@0.2.5 --locked`

## Known Stubs

None.

## Threat Flags

No new threat surface beyond what the plan's threat register already covers.

## Self-Check

- `.github/workflows/release.yml` exists: CONFIRMED
- `softprops/action-gh-release`: CONFIRMED
- `smoke-test.toml` in workflow: CONFIRMED
- `mcp-hub start` in workflow: CONFIRMED
- `cross@0.2.5`: CONFIRMED
- `timeout-minutes`: CONFIRMED
- `needs: [build-linux-x64, build-linux-arm64, build-macos]`: CONFIRMED
- `HOMEBREW_TAP_TOKEN`: CONFIRMED
- `cargo publish`: CONFIRMED
- `cargo publish --dry-run`: CONFIRMED
- `SHA256SUMS`: CONFIRMED
- `CARGO_VERSION` (tag/version check): CONFIRMED
- Commit f4c6fad: CONFIRMED

## Self-Check: PASSED
