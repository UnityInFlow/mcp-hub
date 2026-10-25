# mcp-hub: todo list (2026-10-24)

Source: `docs/reviews/2026-10-24-launch-plan.md`. Tracking issue [#2](https://github.com/UnityInFlow/mcp-hub/issues/2).
Board: [mcp-hub delivery board](https://github.com/orgs/UnityInFlow/projects/13).
Shared conventions and owner defaults: `PORTFOLIO-DECISIONS-2026-10-24.md` in the workspace root.

Working copy: `main` at `0c60172` (2026-06-23), in sync with `origin/main`. Order in the portfolio: this tool is third, after injection-scanner and agent-memory have launched.

## Done 2026-10-24

- [x] Plan written and verified on this tree (226 tests, clippy and fmt clean, release build 5.2 MB arm64)
- [x] Issues #2 to #10 created; #1 folded into #3
- [x] GitHub topics set: `mcp`, `model-context-protocol`, `process-manager`, `rust`, `claude-code`, `cursor`
- [x] Delivery board created and linked to the repo; all issues on it

## First 2 hours (do before anything else)

- [ ] Move `include = [...]` from `[lib]` to `[package]` in `Cargo.toml` (lines 46 to 55); confirm `cargo package --list` has no `.planning/` paths ([#5](https://github.com/UnityInFlow/mcp-hub/issues/5))
- [ ] MANUAL: `cargo publish` `mcp-server-hub` 0.1.2 by hand so the crate stops shipping the planning folder ([#5](https://github.com/UnityInFlow/mcp-hub/issues/5))
- [ ] README "Naming" paragraph, option A: repo and binary `mcp-hub`, crate `mcp-server-hub` ([#5](https://github.com/UnityInFlow/mcp-hub/issues/5))
- [ ] Stop here until injection-scanner and agent-memory have launched

## v0.2.0 launch (about 16 to 24 h)

- [ ] CI and release on GitHub-hosted runners; add `aarch64-apple-darwin` and `x86_64-apple-darwin`; fix the Cyrillic homoglyph at `release.yml:97`; template is agent-memory's `release.yml` ([#3](https://github.com/UnityInFlow/mcp-hub/issues/3), 4 to 6 h, critical path)
- [ ] Same change: rewrite the "CI / Self-Hosted Runners" section of `CLAUDE.md` (it still says "Never use `ubuntu-latest`"), and drop Windows from its distribution lines per shared decision S5 ([#3](https://github.com/UnityInFlow/mcp-hub/issues/3))
- [ ] Dependency refresh one major at a time, `cargo audit` clean, `dependabot.yml` monthly ([#4](https://github.com/UnityInFlow/mcp-hub/issues/4), 2 to 4 h)
- [ ] Bump to 0.2.0, tag, `scripts/verify-published.sh 0.2.0` strict binstall on this Mac ([#7](https://github.com/UnityInFlow/mcp-hub/issues/7), 2 to 3 h)
- [ ] MANUAL: `Formula/mcp-hub.rb` in `UnityInFlow/homebrew-tap`, copied from `agent-memory.rb`, assets `mcp-server-hub-<triple>-v0.2.0.tgz`, `bin.install "mcp-hub"` ([#7](https://github.com/UnityInFlow/mcp-hub/issues/7))
- [ ] README top rewrite from the block in the plan, including "what it does not do"; `docs/demo/demo.tape` with VHS, `docs/demo.gif` ([#8](https://github.com/UnityInFlow/mcp-hub/issues/8), 2 to 3 h)
- [ ] awesome-mcp-servers PRs (punkpeye, wong2), mcpservers.org, one r/ClaudeAI post cross-posted to r/mcp and r/rust ([#9](https://github.com/UnityInFlow/mcp-hub/issues/9), 2 to 3 h)
- [ ] First week: triage, patch-only `v0.2.1`, metrics, retro `docs/reviews/2026-11-xx-week1-retro.md`, go/no-go in #2 ([#10](https://github.com/UnityInFlow/mcp-hub/issues/10), about 3 h)

## Stop rule

Week 1 after the v0.2.0 post: fewer than 25 stars and no external issue or PR and no substantive comment.
Then write the retro, archive with a pointer, leave crates.io and the tap installable. Nothing below before that gate.

## Later, only if demand

- [ ] `gen-config --format codex|gemini` ([#6](https://github.com/UnityInFlow/mcp-hub/issues/6), 2 to 3 h)
- [ ] `mcp-hub attach <name>` stdio shim so clients use the supervised process (v0.3 candidate, a week of work)
- [ ] HTTP / streamable-HTTP health checks; Windsurf / VS Code / Claude Desktop formats; log rotation
- [ ] Not planned: aggregation, namespacing, OAuth (metamcp and mcp-proxy territory)

## Open owner decisions

- D1 release tooling: hand-written `release.yml` (default) or `cargo-dist`. Needed before #3.
- D2 naming: option A (default), B or C. Needed before #5 and #8.
- D3 time box: accept the week-1 thresholds or set your own. Needed before #9.

## 2026-10-25 analysis

Read `docs/reviews/2026-10-25-fable-analysis.md` before starting #3. It turns the plan into a fork: ship v0.2.0 only with an exclusive `mcp-hub attach <name>` shim (about 30 h over three weekends), or do the #5 packaging fix and let the tool rest. Either way, fix the three false statements in the planned README block (`logs -f` is dead, `status` prints JSON in daemon mode, health never restarts) and file the new code findings listed there as issues.
