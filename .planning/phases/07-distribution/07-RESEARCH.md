# Phase 7: Distribution - Research

**Researched:** 2026-04-09
**Domain:** Rust binary distribution — GitHub Actions release CI, cross-compilation, macOS universal binaries, Homebrew tap automation, crates.io publishing
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

- **D-01:** Tag push trigger on `v*` pattern. Push a git tag → CI builds all platforms → creates GitHub Release with binary assets.
- **D-02:** Test gate before build. CI runs `cargo test` + `cargo clippy` on one runner first. Build jobs `needs: test`. Fail fast if tests break.
- **D-03:** Use `cross` crate on self-hosted Linux runners for cross-compilation. `arc-runner-unityinflow` (X64 Linux) builds: `x86_64-unknown-linux-gnu` (native) and `x86_64-pc-windows-gnu` (cross). `orangepi` (ARM64) builds: `aarch64-unknown-linux-gnu` (native).
- **D-04:** Self-hosted Mac runner on the developer's Mac for macOS builds. Labeled with custom label (e.g., `macos-arm64`). Must be online during releases.
- **D-05:** macOS universal binary via `lipo`. Build `aarch64-apple-darwin` and `x86_64-apple-darwin` on the Mac runner, then combine with `lipo -create -output mcp-hub-macos-universal`. Ship as one macOS binary that works on both architectures.
- **D-06:** Total release artifacts: 4 binaries — Linux x86_64, Linux aarch64, Windows x86_64, macOS universal (arm64+x86_64).
- **D-07:** Full automated Homebrew tap at `unityinflow/homebrew-tap` GitHub repo. Release workflow auto-generates `Formula/mcp-hub.rb` with SHA256 checksums of the macOS universal binary, pushes to tap repo.
- **D-08:** User experience: `brew tap unityinflow/tap && brew install mcp-hub`. Formula points to GitHub Release asset URL for the macOS universal binary.
- **D-09:** Publish to crates.io on release. Cargo.toml needs full metadata: name, version, description, license (MIT), repository, homepage, keywords, categories.
- **D-10:** Release workflow includes `cargo publish` step after binaries are uploaded. Requires `CARGO_REGISTRY_TOKEN` secret.

### Claude's Discretion

- Exact Mac runner label name
- Binary naming convention (e.g., `mcp-hub-linux-x86_64.tar.gz` vs `mcp-hub-x86_64-unknown-linux-gnu.tar.gz`)
- Whether to compress as `.tar.gz` (Linux/macOS) and `.zip` (Windows) or all `.tar.gz`
- GitHub Release body template (changelog, checksums, install instructions)
- Whether to add a `release.yml` or extend existing CI workflow
- `cross` version and configuration
- How the Homebrew formula handles version bumps (template with sed/envsubst vs cargo-dist)

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope.

</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DIST-01 | Pre-built binaries for macOS (arm64, x86_64), Linux (x86_64, aarch64), Windows (x86_64) | GitHub Actions matrix with cross + lipo; softprops/action-gh-release@v2 for asset upload |
| DIST-02 | Homebrew formula: `brew install unityinflow/tap/mcp-hub` | Tap repo naming, Formula/ structure, auto-update via bash script + git push or workflow_dispatch trigger |
| DIST-03 | Installable via `cargo install mcp-hub` | Cargo.toml already has required fields; CARGO_REGISTRY_TOKEN secret needed |

</phase_requirements>

---

## Summary

Phase 7 packages the already-implemented mcp-hub binary for distribution across 5 platform variants (macOS arm64, macOS x86_64, Linux x86_64, Linux aarch64, Windows x86_64), ships a macOS universal binary to a Homebrew tap, and publishes the source crate to crates.io.

The core workflow is a GitHub Actions `release.yml` triggered by `v*` tag push. It runs tests first, then fans out to three platform-specific build jobs: (1) a Linux X64 self-hosted runner using `cross` for both `x86_64-unknown-linux-gnu` (native) and `x86_64-pc-windows-gnu` (cross-compiled via Docker container), (2) an ARM64 self-hosted runner (`orangepi`) building `aarch64-unknown-linux-gnu` natively, and (3) the developer's Mac building both macOS arches and combining them with `lipo`. A final release job collects all artifacts, creates the GitHub Release, updates the Homebrew tap formula, and publishes to crates.io.

The biggest structural risk is that `nix` (Unix-only process management crate) is currently an unconditional dependency in `Cargo.toml` — it will fail to compile for `x86_64-pc-windows-gnu`. This must be gated to `cfg(unix)` before the Windows build job can succeed. The source code already uses `#[cfg(unix)]` guards throughout, so only the Cargo.toml declaration needs fixing.

**Primary recommendation:** Write a single `release.yml` with four jobs: `test` → parallel `build-*` matrix jobs → `release` (collect, upload, tap update, crates.io publish). Fix `nix` to be a unix-only dependency first.

---

## Standard Stack

### Core — CI/Release

| Tool | Version | Purpose | Why Standard |
|------|---------|---------|--------------|
| `softprops/action-gh-release` | v2 (latest: v2.6.1) | Create GitHub Release and upload binary assets | Most widely used, supports glob file patterns, handles release body |
| `actions/upload-artifact` | v4 | Pass build outputs between jobs | Required for fan-out build → release job pattern |
| `actions/download-artifact` | v4 | Collect artifacts in release job | Pairs with upload-artifact v4 |
| `actions/checkout` | v4 | Checkout repo in each job | Standard |
| `cross` | 0.2.5 (latest on crates.io) | Cross-compile Rust to foreign targets using Docker containers | Only practical way to cross-compile nix-dependent code to Windows/ARM from a different arch |

[VERIFIED: npm registry / cargo search output — cross = "0.2.5", softprops/action-gh-release latest v2.6.1 per GitHub Marketplace fetch]

### Core — macOS Universal Binary

| Tool | Version | Purpose | Why Standard |
|------|---------|---------|--------------|
| `lipo` | System tool (macOS) | Combine arm64 + x86_64 Mach-O binaries into a universal binary | Apple-native tool, zero overhead, produces a single file |

[VERIFIED: lipo present at /usr/bin/lipo on macOS via Bash tool]

### Core — Homebrew Tap

| Component | Notes | Purpose |
|-----------|-------|---------|
| `unityinflow/homebrew-tap` GitHub repo | Repo must be named `homebrew-tap` for `brew tap unityinflow/tap` shortform to work | Hosts the Formula/mcp-hub.rb file |
| `Formula/mcp-hub.rb` | Ruby file with class McpHub, on_macos block, install block, test block | Formula Homebrew reads when user runs `brew install` |
| Cross-repo PAT secret | GitHub Personal Access Token with `repo` + `workflow` scopes, stored as `HOMEBREW_TAP_TOKEN` | Required for main repo release workflow to push to tap repo |

[CITED: https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap — repo naming requirement verified]

### Core — crates.io

| Requirement | Status | Notes |
|-------------|--------|-------|
| `CARGO_REGISTRY_TOKEN` secret | Must be added to repo secrets | Obtained from crates.io/settings/tokens |
| Cargo.toml metadata | Already present | name, version, description, license, repository, keywords, categories all present |
| `cargo publish --dry-run` | Run first in CI | Catches packaging errors without publishing |

[VERIFIED: Cargo.toml read directly — all required crates.io fields are present]

### Installation Commands

```bash
# Install cross on the Linux runners (done once, or pre-installed on runner image)
cargo install cross --locked

# Install Rust targets on Mac runner
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

---

## Architecture Patterns

### Recommended Workflow Structure

```
.github/workflows/
├── ci.yml           # Existing: runs on push/PR, test + clippy + fmt
└── release.yml      # New: runs on v* tag push only
```

Separating release from CI avoids polluting normal PR runs with slow multi-platform builds.

### Pattern 1: Fan-Out Build Matrix → Collect → Release

```
[test job]
    │ needs: test
    ├─── [build-linux-x64]   → upload-artifact: linux-x64/
    ├─── [build-linux-arm64] → upload-artifact: linux-arm64/
    └─── [build-macos]       → upload-artifact: macos-universal/
                                    │ needs: all three build-*
                                    ▼
                              [release]
                              download-artifact (all)
                              softprops/action-gh-release (upload all)
                              update homebrew tap
                              cargo publish
```

**What:** Test gates the entire pipeline. Three independent build jobs run in parallel on dedicated runners. A single release job waits for all three, then performs upload, tap update, and crates.io publish atomically.

**When to use:** Always for multi-platform release pipelines — jobs share no state, artifacts are the handoff mechanism.

### Pattern 2: Binary Naming Convention

Recommendation (Claude's discretion): use human-readable names, not triple names.

```
mcp-hub-linux-x86_64.tar.gz
mcp-hub-linux-aarch64.tar.gz
mcp-hub-windows-x86_64.zip          # .zip for Windows, .tar.gz for Unix
mcp-hub-macos-universal.tar.gz
```

Rationale: Users downloading from GitHub Releases page see these names. Triple names (`x86_64-unknown-linux-gnu`) are harder to parse. The Homebrew formula references the exact filename, so consistency matters.

### Pattern 3: Release Workflow Job Definitions

```yaml
# Source: pattern verified from softprops/action-gh-release docs + cross-rs docs

on:
  push:
    tags: ['v*']

jobs:
  test:
    runs-on: [arc-runner-unityinflow]
    steps:
      - uses: actions/checkout@v4
      - run: cargo test
      - run: cargo clippy -- -D warnings
      - run: cargo fmt --check

  build-linux-x64:
    needs: test
    runs-on: [arc-runner-unityinflow]
    steps:
      - uses: actions/checkout@v4
      - name: Install cross
        run: cargo install cross --locked
      - name: Build x86_64 Linux (native)
        run: cross build --release --target x86_64-unknown-linux-gnu
      - name: Build Windows x86_64 (cross)
        run: cross build --release --target x86_64-pc-windows-gnu
      - name: Package
        run: |
          TAG=${GITHUB_REF##refs/tags/}
          tar czf mcp-hub-linux-x86_64.tar.gz -C target/x86_64-unknown-linux-gnu/release mcp-hub
          zip mcp-hub-windows-x86_64.zip -j target/x86_64-pc-windows-gnu/release/mcp-hub.exe
      - uses: actions/upload-artifact@v4
        with:
          name: linux-x64-windows
          path: |
            mcp-hub-linux-x86_64.tar.gz
            mcp-hub-windows-x86_64.zip

  build-linux-arm64:
    needs: test
    runs-on: [orangepi]
    steps:
      - uses: actions/checkout@v4
      - name: Build aarch64 Linux (native)
        run: cargo build --release --target aarch64-unknown-linux-gnu
      - name: Package
        run: tar czf mcp-hub-linux-aarch64.tar.gz -C target/aarch64-unknown-linux-gnu/release mcp-hub
      - uses: actions/upload-artifact@v4
        with:
          name: linux-arm64
          path: mcp-hub-linux-aarch64.tar.gz

  build-macos:
    needs: test
    runs-on: [macos-arm64]   # developer's Mac runner — label TBD (Claude's discretion)
    steps:
      - uses: actions/checkout@v4
      - name: Add x86_64 target
        run: rustup target add x86_64-apple-darwin
      - name: Build arm64
        run: cargo build --release --target aarch64-apple-darwin
      - name: Build x86_64
        run: cargo build --release --target x86_64-apple-darwin
      - name: Create universal binary
        run: |
          lipo -create \
            target/aarch64-apple-darwin/release/mcp-hub \
            target/x86_64-apple-darwin/release/mcp-hub \
            -output mcp-hub-universal
          tar czf mcp-hub-macos-universal.tar.gz mcp-hub-universal
      - uses: actions/upload-artifact@v4
        with:
          name: macos-universal
          path: mcp-hub-macos-universal.tar.gz

  release:
    needs: [build-linux-x64, build-linux-arm64, build-macos]
    runs-on: [arc-runner-unityinflow]
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          merge-multiple: true
          path: artifacts/
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: artifacts/**
          generate_release_notes: true
      - name: Update Homebrew tap
        # see Homebrew Tap section below
      - name: Publish to crates.io
        run: cargo publish --no-verify
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

### Pattern 4: Homebrew Formula Auto-Update

The `mislav/bump-homebrew-formula-action` does NOT support multi-arch formulas with per-arch SHA256 blocks. Since D-05 uses a single macOS universal binary (one asset, one SHA256), the formula has a single sha256 line. This makes `mislav/bump-homebrew-formula-action` viable. Alternatively, a bash script approach with `sed` is more transparent and requires no third-party action.

**Recommended approach (bash script, no third-party action):**

```bash
# In release job, after softprops creates the release:
TAG=${GITHUB_REF##refs/tags/}
VERSION=${TAG#v}

# Compute SHA256 of the macOS universal archive
SHA256=$(curl -sL "https://github.com/UnityInFlow/mcp-hub/releases/download/${TAG}/mcp-hub-macos-universal.tar.gz" \
  | shasum -a 256 | awk '{print $1}')

# Clone tap repo and update formula
git clone "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/unityinflow/homebrew-tap.git" tap-repo
cat > tap-repo/Formula/mcp-hub.rb << FORMULA
class McpHub < Formula
  desc "PM2 for MCP servers — manage, monitor, and configure your MCP servers"
  homepage "https://github.com/UnityInFlow/mcp-hub"
  version "${VERSION}"
  license "MIT"

  on_macos do
    url "https://github.com/UnityInFlow/mcp-hub/releases/download/${TAG}/mcp-hub-macos-universal.tar.gz"
    sha256 "${SHA256}"
  end

  def install
    bin.install "mcp-hub-universal" => "mcp-hub"
  end

  test do
    system "#{bin}/mcp-hub", "--version"
  end
end
FORMULA

cd tap-repo
git config user.name "github-actions[bot]"
git config user.email "github-actions[bot]@users.noreply.github.com"
git add Formula/mcp-hub.rb
git commit -m "chore: bump mcp-hub to ${VERSION}"
git push
```

[CITED: https://josh.fail/2023/automate-updating-custom-homebrew-formulae-with-github-actions/ — cross-repo workflow trigger pattern; https://kristoffer.dev/blog/guide-to-creating-your-first-homebrew-tap/ — Formula Ruby class structure]

### Recommended Project Structure

```
.github/
└── workflows/
    ├── ci.yml                     # existing test/lint workflow
    └── release.yml                # new: tag-triggered release pipeline

unityinflow/homebrew-tap (separate repo)
└── Formula/
    └── mcp-hub.rb                 # auto-generated by release workflow

Cargo.toml
└── [target.'cfg(unix)'.dependencies]
    └── nix = ...                  # MUST be unix-gated before Windows build
```

### Anti-Patterns to Avoid

- **Using `ubuntu-latest` as runner:** CLAUDE.md explicitly prohibits this. Always use `[arc-runner-unityinflow]` or `[orangepi]`.
- **Running `cargo build` without `--target` on the ARM runner:** The orangepi runner is ARM64 but `aarch64-unknown-linux-gnu` is a distinct target triple — always specify `--target` explicitly.
- **Unconditional `nix` dependency:** Will cause Windows build to fail with `error[E0433]: failed to resolve`. Must be `[target.'cfg(unix)'.dependencies]`.
- **Publishing to crates.io before verifying `cargo publish --dry-run`:** The 10MB package size limit and missing README will cause publish failures. Include `--dry-run` step.
- **Hardcoding the Mac runner label:** The label is "Claude's discretion" — use a GitHub Actions variable or environment variable so it can be changed without editing the workflow.
- **Running `cargo publish` with `--locked`:** crates.io rejects packages published with `--locked` in some configurations. Use `--no-verify` only if `--dry-run` has already validated.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Cross-platform compilation | Custom Docker images or rustup target add + linker flags | `cross` crate (0.2.5) | cross ships pre-built Docker images with correct linkers, sysroots, and C libraries for each target; hand-rolling this takes days |
| GitHub Release creation | `gh release create` scripting | `softprops/action-gh-release@v2` | Handles race conditions, asset replacement, release body, tag validation |
| Artifact passing between jobs | Uploading to external storage | `actions/upload-artifact@v4` + `actions/download-artifact@v4` | Built-in, no token required, automatic cleanup |
| macOS universal binary (arm64+x86_64) | Fat binary construction logic | `lipo -create` | Apple-native, one command, supported by all macOS versions |

**Key insight:** Binary distribution is 90% CI plumbing. The actual Rust build is trivial — the complexity is in artifact handoff, cross-compilation toolchain setup, and Homebrew formula lifecycle.

---

## Common Pitfalls

### Pitfall 1: `nix` Crate Breaks Windows Build

**What goes wrong:** `cross build --target x86_64-pc-windows-gnu` fails immediately because `nix` 0.29 only compiles on Unix targets. Error: `error[E0433]: failed to resolve: could not find 'nix'`.
**Why it happens:** `nix` is currently listed as an unconditional dependency in Cargo.toml. The source code guards its usage behind `#[cfg(unix)]` blocks, but Cargo still tries to compile the crate for all targets.
**How to avoid:** Change Cargo.toml to `[target.'cfg(unix)'.dependencies]` for `nix`. This makes Cargo skip the crate for non-Unix targets.
**Warning signs:** Any `cross build --target x86_64-pc-windows-gnu` failing on dependency resolution (not linker) errors.

[VERIFIED: nix crate GitHub repo confirms Unix-only; source grep confirms nix imports are only within #[cfg(unix)] blocks in the source, but Cargo.toml still has it unconditional]

### Pitfall 2: `cross` Requires Docker on the Linux Runners

**What goes wrong:** `cross build` fails with "Docker daemon is not running" or permission errors on the ARC (Kubernetes) runners if privileged mode is not enabled.
**Why it happens:** `cross` uses Docker containers as cross-compilation environments. ARC runners run in Kubernetes pods; Docker-in-Docker requires `securityContext.privileged: true`.
**How to avoid:** Either (a) verify the `arc-runner-unityinflow` pods have privileged mode enabled, or (b) set `CROSS_CONTAINER_IN_CONTAINER=true` and ensure the Docker socket is mounted. Alternatively, install cross-compilation toolchains directly on the runner and use `cargo build` with `CARGO_TARGET_*_LINKER` env vars.
**Fallback:** For `aarch64-unknown-linux-gnu` on the orangepi runner (native ARM64), `cross` is not needed — use plain `cargo build --target aarch64-unknown-linux-gnu` after installing the target with `rustup target add`.

[CITED: https://www.stepsecurity.io/blog/how-to-use-docker-in-actions-runner-controller-runners-securelly — ARC privileged mode requirement]

### Pitfall 3: Mac Runner Must Be Online During Release

**What goes wrong:** The `build-macos` job hangs indefinitely (up to 6 hours) if the developer's Mac is offline when a release tag is pushed.
**Why it happens:** Self-hosted runners that are offline do not queue-reject jobs; GitHub Actions waits for them.
**How to avoid:** Add `continue-on-error: true` to the `build-macos` job (per CONTEXT.md specifics note). The Linux/Windows artifacts still land in the release. The macOS universal binary can be added later by re-running just that job.
**Warning signs:** Release job waiting >15 minutes with no build-macos output.

[ASSUMED: `continue-on-error: true` behavior for self-hosted runners — based on GitHub Actions documentation knowledge, not verified via tool in this session]

### Pitfall 4: Homebrew Formula References Wrong Binary Name Inside Archive

**What goes wrong:** `brew install mcp-hub` downloads and extracts the archive but `brew` cannot find the binary to install, giving "No such file or directory".
**Why it happens:** The `install` block uses `bin.install "mcp-hub"` but the binary inside the archive is named `mcp-hub-universal` (the name given to the `lipo` output).
**How to avoid:** Either (a) name the lipo output `mcp-hub` (simplest), or (b) use `bin.install "mcp-hub-universal" => "mcp-hub"` in the formula's install block. The formula template above uses option (b).
**Warning signs:** `brew install` succeeds but `which mcp-hub` returns nothing.

### Pitfall 5: crates.io Name Reservation

**What goes wrong:** `cargo publish` fails with "crate name already taken" if someone else has registered `mcp-hub` on crates.io.
**Why it happens:** crates.io is first-come-first-served.
**How to avoid:** Check `cargo search mcp-hub` and visit `https://crates.io/crates/mcp-hub` before the release workflow is written. If taken, the crate must be published under a scoped name or the owner contacted.
**Warning signs:** `cargo publish` fails on "already registered".

[CITED: https://doc.rust-lang.org/cargo/reference/publishing.html — first-come-first-served crate name policy]

### Pitfall 6: `cargo publish` Race With Binary Upload

**What goes wrong:** `cargo publish` succeeds but then a user runs `cargo install mcp-hub` before the binary releases are up, getting a build that fails on their machine if they don't have all system dependencies.
**Why it happens:** `cargo publish` (source) and binary upload are separate operations.
**How to avoid:** Run `cargo publish` last in the release job, after all binary assets are uploaded and the Homebrew tap is updated. The `cargo install` path always builds from source, so it's acceptable — the user must have a Rust toolchain.

### Pitfall 7: `actions/download-artifact@v4` Merge Behavior

**What goes wrong:** All artifacts land in a flat directory and filenames collide, or artifacts from different jobs overwrite each other.
**Why it happens:** v4 `merge-multiple: true` merges all artifact directories into one. If two jobs produce files with the same name, one overwrites the other.
**How to avoid:** Give each build job's artifact a unique name (`name: linux-x64-windows`, `name: linux-arm64`, `name: macos-universal`). Download with `merge-multiple: true` to get them all in one flat directory. Each file has a unique name (`mcp-hub-linux-x86_64.tar.gz`, etc.) so no collision.

---

## Code Examples

### Cargo.toml — nix Dependency Fix

```toml
# Source: cargo Cargo.toml platform-specific dependencies spec
# BEFORE (broken for Windows):
# nix = { version = "0.29", features = ["signal", "process"] }

# AFTER (correct):
[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["signal", "process"] }
```

### Cargo.toml — Metadata for crates.io (current state — already complete)

```toml
# Source: verified by reading Cargo.toml directly
[package]
name = "mcp-hub"
version = "0.0.1"
edition = "2021"
description = "PM2 for MCP servers — manage, monitor, and configure your MCP servers"
license = "MIT"
repository = "https://github.com/UnityInFlow/mcp-hub"
keywords = ["mcp", "process-manager", "ai", "developer-tools"]
categories = ["command-line-utilities", "development-tools"]
```

All required crates.io fields are already present. No changes needed beyond the `nix` gate fix.

### Homebrew Formula — mcp-hub.rb Template

```ruby
# Source: pattern from https://kristoffer.dev/blog/guide-to-creating-your-first-homebrew-tap/
# and https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap
class McpHub < Formula
  desc "PM2 for MCP servers — manage, monitor, and configure your MCP servers"
  homepage "https://github.com/UnityInFlow/mcp-hub"
  version "VERSION_PLACEHOLDER"
  license "MIT"

  on_macos do
    url "https://github.com/UnityInFlow/mcp-hub/releases/download/vVERSION_PLACEHOLDER/mcp-hub-macos-universal.tar.gz"
    sha256 "SHA256_PLACEHOLDER"
  end

  def install
    # lipo output should be named "mcp-hub" for clean install,
    # OR use the rename form: bin.install "mcp-hub-universal" => "mcp-hub"
    bin.install "mcp-hub"
  end

  test do
    system "#{bin}/mcp-hub", "--version"
  end
end
```

### lipo Universal Binary Creation

```bash
# Source: Apple documentation on lipo, verified lipo present at /usr/bin/lipo on macOS
lipo -create \
  target/aarch64-apple-darwin/release/mcp-hub \
  target/x86_64-apple-darwin/release/mcp-hub \
  -output target/release/mcp-hub
# Then archive:
tar czf mcp-hub-macos-universal.tar.gz -C target/release mcp-hub
```

### Cargo Publish in CI

```bash
# Source: https://doc.rust-lang.org/cargo/reference/publishing.html
# Dry run first (catches packaging errors)
cargo publish --dry-run

# Actual publish (CARGO_REGISTRY_TOKEN must be set in env)
cargo publish
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `actions/upload-release-asset` (deprecated) | `softprops/action-gh-release@v2` | 2022 | upload-release-asset requires separate create-release step; action-gh-release does both |
| `actions/upload-artifact@v3` | `actions/upload-artifact@v4` | 2024 | v4 is 10x faster, supports `merge-multiple` on download |
| Per-arch macOS binaries (arm64 + x86_64) | Single universal binary via `lipo` | 2021 (Apple Silicon) | One download for all Macs; Homebrew formula needs only one SHA256 |
| `mislav/bump-homebrew-formula-action` | Bash script + git push | ongoing | The action doesn't support multi-SHA256 formulas; bash is simpler and more transparent for single-asset formulas |

**Deprecated/outdated:**
- `actions/create-release` + `actions/upload-release-asset`: Both deprecated; replaced by `softprops/action-gh-release`.
- `actions/upload-artifact@v3`: Still functional but v4 is the current standard; v3 has slower upload speeds.

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `continue-on-error: true` on `build-macos` allows Linux/Windows jobs to complete even if Mac is offline | Common Pitfalls #3 | Mac job stalls entire release; release never completes. Mitigation: test with Mac offline before shipping workflow |
| A2 | The `arc-runner-unityinflow` Kubernetes pods have Docker available (privileged mode enabled) for `cross` | Common Pitfalls #2 | Windows cross-compilation fails; workaround is to install cross-linker toolchain directly on runner |
| A3 | `mcp-hub` crate name is not yet registered on crates.io | Common Pitfalls #5 | Must rename or contact owner; delays DIST-03 |

---

## Open Questions

1. **Mac runner label**
   - What we know: Decision D-04 says "labeled with custom label (e.g., `macos-arm64`)". This is Claude's discretion.
   - What's unclear: The actual label name the developer configures when registering their Mac as a self-hosted runner.
   - Recommendation: Use `macos-arm64` as the label in the workflow, document that the Mac runner must be registered with exactly this label. Include it as a note in the release.yml comments.

2. **Docker availability on `arc-runner-unityinflow`**
   - What we know: `cross` requires Docker (or Podman). ARC runners need privileged mode for Docker-in-Docker.
   - What's unclear: Whether the existing Hetzner ARC runners have Docker available and whether privileged mode is enabled.
   - Recommendation: Wave 0 task — SSH to arc-runner and run `docker info`. If unavailable, use `cargo zigbuild` or install cross-compilation toolchains directly (gcc-multilib, gcc-aarch64-linux-gnu, mingw-w64).

3. **Binary name inside macOS archive**
   - What we know: `lipo` output can be named anything; Homebrew's `install` block maps archive-internal name to `bin/mcp-hub`.
   - What's unclear: Whether to name the lipo output `mcp-hub` (simplest) or `mcp-hub-universal` (clearer provenance).
   - Recommendation: Name the lipo output `mcp-hub` directly (avoid the rename complexity in the formula).

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| `lipo` | DIST-01: macOS universal binary | ✓ | /usr/bin/lipo (macOS system) | None needed — available on all macOS |
| `Docker` | DIST-01: `cross` cross-compilation | ✓ (dev machine) | 28.5.1 | Unknown on ARC runners — see Open Questions #2 |
| `gh` CLI | Release creation | ✓ | 2.55.0 | softprops action handles it in CI |
| `cross` crate | DIST-01: Windows + Linux cross-build | Not installed | — (0.2.5 on crates.io) | `cargo install cross --locked` in CI |
| crates.io account | DIST-03 | [ASSUMED] exists | — | Create account at crates.io |
| `homebrew-tap` repo | DIST-02 | [ASSUMED] does not exist yet | — | Create as `unityinflow/homebrew-tap` |
| `CARGO_REGISTRY_TOKEN` secret | DIST-03 | Not set (no CI yet) | — | Create token at crates.io/settings/tokens, add to repo secrets |
| `HOMEBREW_TAP_TOKEN` secret | DIST-02 | Not set | — | Create PAT with `repo` + `workflow` scopes |

**Missing dependencies with no fallback:**
- None — all have workarounds or straightforward setup steps.

**Missing dependencies with fallback:**
- Docker on ARC runners: if unavailable, install cross-compilation toolchains natively (see Open Questions #2).
- `homebrew-tap` repo: must be created as Wave 0 task before release job can push the formula.

---

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Rust built-in + assert_cmd 2.x |
| Config file | none (Cargo.toml dev-dependencies) |
| Quick run command | `cargo test` |
| Full suite command | `cargo test --all` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| DIST-01 | Binary compiles for all 5 platform variants | CI build job (not unit test) | `cross build --release --target x86_64-pc-windows-gnu` | CI job — no test file |
| DIST-01 | `mcp-hub --version` passes on each platform | Smoke test in release job | `./mcp-hub --version` in CI | Manual in CI step |
| DIST-02 | Homebrew formula is valid Ruby | Lint | `brew audit --new Formula/mcp-hub.rb` | Manual/CI in tap repo |
| DIST-02 | `brew install unityinflow/tap/mcp-hub` installs binary | Integration test | Manual (requires macOS + Homebrew) | Manual |
| DIST-03 | `cargo publish --dry-run` succeeds | Smoke | `cargo publish --dry-run` | CI step |

### Sampling Rate

- **Per task commit:** `cargo test` (existing suite, ~226 tests)
- **Per wave merge:** `cargo test --all` + `cargo clippy -- -D warnings` + `cargo fmt --check`
- **Phase gate:** All CI jobs green on the release tag before `/gsd-verify-work`

### Wave 0 Gaps

- [ ] `unityinflow/homebrew-tap` repo must exist before release workflow can push formula
- [ ] Mac runner must be registered with label `macos-arm64` (or chosen label) before `build-macos` job can run
- [ ] `CARGO_REGISTRY_TOKEN` and `HOMEBREW_TAP_TOKEN` secrets must be added to repo settings
- [ ] Docker availability on `arc-runner-unityinflow` must be verified

---

## Project Constraints (from CLAUDE.md)

Directives from `./CLAUDE.md` that affect this phase:

| Directive | Impact on Phase 7 |
|-----------|-------------------|
| Never use `ubuntu-latest` | All jobs must use `[arc-runner-unityinflow]`, `[orangepi]`, or the Mac runner label |
| `cargo clippy -- -D warnings` must pass | Test gate job must run clippy, not just `cargo test` |
| `cargo fmt` before every commit | Test gate should include `cargo fmt --check` |
| No `unwrap()` in production code | Verify binary passes clippy before release |
| Pre-built binaries for macOS (arm64/x86_64), Linux (x86_64/aarch64), Windows | Direct requirement; all 5 variants must appear in release assets |
| Distribution: pre-built binaries + Homebrew tap + `cargo install` | All three channels required for v0.0.1 acceptance |

---

## Sources

### Primary (HIGH confidence)
- Cargo.toml (read directly) — confirmed all crates.io metadata fields present, confirmed `nix` is unconditional dependency
- Source code grep (src/*.rs) — confirmed `nix` usage is gated by `#[cfg(unix)]` in all files
- `lipo` at /usr/bin/lipo — confirmed present on macOS dev machine
- Docker 28.5.1 at /usr/local/bin/docker — confirmed on dev machine
- `cargo search cross` output — confirmed cross = "0.2.5" on crates.io
- `gh version 2.55.0` — confirmed gh CLI available
- [https://github.com/nix-rust/nix](https://github.com/nix-rust/nix) — confirmed nix is Unix-only
- [https://doc.rust-lang.org/cargo/reference/publishing.html](https://doc.rust-lang.org/cargo/reference/publishing.html) — crates.io requirements verified
- [https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap) — tap naming and formula location

### Secondary (MEDIUM confidence)
- [https://github.com/softprops/action-gh-release](https://github.com/softprops/action-gh-release) — v2.6.1 confirmed, file glob upload pattern
- [https://kristoffer.dev/blog/guide-to-creating-your-first-homebrew-tap/](https://kristoffer.dev/blog/guide-to-creating-your-first-homebrew-tap/) — Ruby formula structure with on_macos/on_linux blocks
- [https://josh.fail/2023/automate-updating-custom-homebrew-formulae-with-github-actions/](https://josh.fail/2023/automate-updating-custom-homebrew-formulae-with-github-actions/) — cross-repo tap update workflow
- [https://github.com/mislav/bump-homebrew-formula-action](https://github.com/mislav/bump-homebrew-formula-action) — confirmed does NOT support multi-SHA256 arch formulas

### Tertiary (LOW confidence)
- `continue-on-error: true` behavior for offline self-hosted Mac runner — inferred from GitHub Actions docs knowledge, not tested

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — versions verified via cargo search and GitHub Marketplace fetch
- Architecture: HIGH — workflow pattern is standard for Rust releases, verified across multiple sources
- Pitfalls: HIGH (nix issue VERIFIED), MEDIUM (Docker on ARC runners ASSUMED), HIGH (Homebrew binary name)
- Homebrew formula: MEDIUM-HIGH — structure verified via official docs + blog; auto-update bash pattern verified

**Research date:** 2026-04-09
**Valid until:** 2026-05-09 (GitHub Actions action versions update frequently; verify softprops@v2 is still latest before executing)
