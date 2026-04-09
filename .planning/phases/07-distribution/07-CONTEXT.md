# Phase 7: Distribution - Context

**Gathered:** 2026-04-09
**Status:** Ready for planning

<domain>
## Phase Boundary

Produce pre-built binaries for all target platforms via GitHub Actions CI, publish to Homebrew tap with auto-generated formula, enable `cargo install` via crates.io, and verify all binaries work end-to-end.

Requirements: DIST-01, DIST-02, DIST-03.

</domain>

<decisions>
## Implementation Decisions

### CI release workflow
- **D-01:** Tag push trigger on `v*` pattern. Push a git tag → CI builds all platforms → creates GitHub Release with binary assets.
- **D-02:** Test gate before build. CI runs `cargo test` + `cargo clippy` on one runner first. Build jobs `needs: test`. Fail fast if tests break.

### Build matrix & cross-compilation
- **D-03:** Use `cross` crate on self-hosted Linux runners for cross-compilation. `arc-runner-unityinflow` (X64 Linux) builds: `x86_64-unknown-linux-gnu` (native) and `x86_64-pc-windows-gnu` (cross). `orangepi` (ARM64) builds: `aarch64-unknown-linux-gnu` (native).
- **D-04:** Self-hosted Mac runner on the developer's Mac for macOS builds. Labeled with custom label (e.g., `macos-arm64`). Must be online during releases.
- **D-05:** macOS universal binary via `lipo`. Build `aarch64-apple-darwin` and `x86_64-apple-darwin` on the Mac runner, then combine with `lipo -create -output mcp-hub-macos-universal`. Ship as one macOS binary that works on both architectures.
- **D-06:** Total release artifacts: 4 binaries — Linux x86_64, Linux aarch64, Windows x86_64, macOS universal (arm64+x86_64).

### Homebrew tap
- **D-07:** Full automated Homebrew tap at `unityinflow/homebrew-tap` GitHub repo. Release workflow auto-generates `Formula/mcp-hub.rb` with SHA256 checksums of the macOS universal binary, pushes to tap repo.
- **D-08:** User experience: `brew tap unityinflow/tap && brew install mcp-hub`. Formula points to GitHub Release asset URL for the macOS universal binary.

### cargo install / crates.io
- **D-09:** Publish to crates.io on release. Cargo.toml needs full metadata: name, version, description, license (MIT), repository, homepage, keywords, categories.
- **D-10:** Release workflow includes `cargo publish` step after binaries are uploaded. Requires `CARGO_REGISTRY_TOKEN` secret.

### Claude's Discretion
- Exact Mac runner label name
- Binary naming convention (e.g., `mcp-hub-linux-x86_64.tar.gz` vs `mcp-hub-x86_64-unknown-linux-gnu.tar.gz`)
- Whether to compress binaries as `.tar.gz` (Linux/macOS) and `.zip` (Windows) or all `.tar.gz`
- GitHub Release body template (changelog, checksums, install instructions)
- Whether to add a `release.yml` or extend existing CI workflow
- `cross` version and configuration
- How the Homebrew formula handles version bumps (template with sed/envsubst vs cargo-dist)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Project spec
- `07-mcp-hub.md` — Full feature spec, distribution section
- `.planning/PROJECT.md` — Project vision, constraints, CI runner details

### Requirements
- `.planning/REQUIREMENTS.md` — DIST-01, DIST-02, DIST-03 definitions

### CI runner details (from CLAUDE.md)
- `CLAUDE.md` — Self-hosted runner labels: `arc-runner-unityinflow` (X64), `orangepi` (ARM64). Never use `ubuntu-latest`.

### Existing code
- `Cargo.toml` — Current package metadata (needs enhancement for crates.io)
- `.github/workflows/` — Existing CI workflows (if any)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `Cargo.toml` — Already has name, version, edition. Needs description, license, repository, keywords, categories for crates.io.
- Existing test suite (226 tests) — Reuse as release gate.

### Established Patterns
- Self-hosted runners with custom labels (from CLAUDE.md)
- `cargo build --release` for optimized binaries
- `cargo clippy -- -D warnings` as quality gate

### Integration Points
- GitHub Actions workflow file (`.github/workflows/release.yml`) — new file
- Homebrew tap repo (`unityinflow/homebrew-tap`) — new external repo
- crates.io — new publishing target
- GitHub Releases — asset upload via `gh release create` or `actions/upload-release-asset`

</code_context>

<specifics>
## Specific Ideas

- The release should feel like a one-command operation: `git tag v0.1.0 && git push origin v0.1.0` → everything else is automated
- macOS universal binary via `lipo` is the cleanest approach — one download for all Macs
- Homebrew formula should auto-update on every release, not require manual SHA256 updates
- The Mac runner being offline should not block Linux/Windows builds — those should complete even if Mac is unavailable (use `continue-on-error` or separate jobs)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 07-distribution*
*Context gathered: 2026-04-09*
