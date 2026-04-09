# Phase 7: Distribution - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-09
**Phase:** 07-distribution
**Areas discussed:** CI release workflow, Cross-compilation approach, Homebrew formula, cargo install readiness

---

## CI release workflow

| Option | Description | Selected |
|--------|-------------|----------|
| Tag push | Push v* tag triggers release. Standard Rust pattern. | ✓ |
| Manual dispatch | workflow_dispatch with version input. | |
| Both | Tag push AND manual dispatch. | |

**User's choice:** Tag push
**Notes:** Standard pattern. git tag v0.1.0 && git push origin v0.1.0.

| Option | Description | Selected |
|--------|-------------|----------|
| Yes, test first | cargo test + clippy before build. Fail fast. | ✓ |
| Skip tests in release | Assume tests passed before tagging. | |

**User's choice:** Test first

---

## Cross-compilation approach

| Option | Description | Selected |
|--------|-------------|----------|
| cross crate on self-hosted | cross for Linux/Windows on existing runners. | ✓ |
| GitHub-hosted for macOS | Pay $0.08/min for macos-latest. | |
| You decide | Claude's discretion. | |

**User's choice:** cross crate on self-hosted

| Option | Description | Selected |
|--------|-------------|----------|
| GitHub-hosted macos-latest | Pay for GitHub macOS runners. | |
| Skip macOS for v0.1.0 | Initially selected, then revised. | |
| Self-hosted runner on your Mac | Install Actions runner on dev Mac. Free, automated. | ✓ |

**User's choice:** Self-hosted Mac runner
**Notes:** Universal binary via lipo (arm64+x86_64). Mac must be online during releases.

---

## Homebrew formula

| Option | Description | Selected |
|--------|-------------|----------|
| Full Homebrew tap | Auto-generated formula, pushed to unityinflow/homebrew-tap. | ✓ |
| Defer to v0.2.0 | Skip Homebrew, users download or cargo install. | |
| Manual formula only | Create tap manually after first release. | |

**User's choice:** Full automated Homebrew tap
**Notes:** Release workflow generates formula with SHA256, pushes to tap repo.

---

## cargo install readiness

| Option | Description | Selected |
|--------|-------------|----------|
| Publish to crates.io | Full metadata, cargo publish in release workflow. | ✓ |
| Not yet | Skip crates.io, GitHub Release only. | |

**User's choice:** Publish to crates.io
**Notes:** Needs CARGO_REGISTRY_TOKEN secret.

---

## Claude's Discretion

- Mac runner label name
- Binary naming convention
- Compression format (.tar.gz vs .zip)
- GitHub Release body template
- Workflow file structure
- cross configuration
- Homebrew formula version bump mechanism

## Deferred Ideas

None — discussion stayed within phase scope
