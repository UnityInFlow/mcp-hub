# mcp-hub: revive-and-launch plan (2026-10-24)

Tracking issue: [UnityInFlow/mcp-hub#2](https://github.com/UnityInFlow/mcp-hub/issues/2).
Repo state at time of writing: `main` clean, in sync with `origin/main` at `0c60172` (2026-06-23).
All file:line references are against that commit.

## Where it stands

1. PM2-style supervisor for local stdio MCP servers: TOML config, `start/stop/restart/status/logs/reload`, MCP `ping` health checks with Healthy/Degraded/Failed states, exponential-backoff restarts, unified logs, daemon mode over a Unix socket, embedded web UI (axum + askama, HTMX inlined via `include_str!` in `src/web/assets.rs:8-14`), `init` wizard, `gen-config --format claude|cursor` (`src/cli.rs:65-69`).
2. Size: 5,414 LOC in `src/`, 4,457 in `tests/`, 649 in templates/static. `cargo test` today: **226 passed, 0 failed** across 20 test binaries (32 lib unit + 40 bin unit + 154 integration). The "26 tests" figure in the portfolio notes counts test *files*, not tests.
3. Builds and lints clean on this Mac: `cargo build --release` gives a 5.2 MB arm64 Mach-O in 48 s; `cargo fmt --check` and `cargo clippy -- -D warnings` pass. One manifest warning (`unused manifest key: lib.include`): see step 3, it is a real packaging bug.
4. Published: crates.io `mcp-server-hub` 0.1.1 (47 downloads; `mcp-hub` on crates.io is gleb-chipiga's unrelated aggregation server, 0.3.0, 582 downloads). GitHub releases v0.1.0 (2026-04-09) and v0.1.1 (2026-06-23) with four Linux `.tgz` assets only. 0 stars, 0 forks, no topics, 1 open issue (#1, a homoglyph in a comment).
5. CI is dead, not flaky: `ci.yml:13` and every `runs-on` in `release.yml` (lines 32, 74, 119, 233, 329) name self-hosted runners; the org runner group no longer admits public repos (see `UnityInFlow/agent-memory` release.yml header, enforced 2026-07-09), so all CI runs since 2026-06-22 ended `cancelled` after the 24 h queue timeout. The v0.1.1 release run succeeded only because `orangepi` was still reachable that day.
6. macOS is explicitly deferred: `release.yml:109` ("macOS is deferred to v2 ... no macOS target here"); README lines 49-53 say so. The original plan (`.planning/phases/07-distribution/07-CONTEXT.md:24-26`) wanted a self-hosted Mac runner plus `lipo`; neither happened. Hand-written workflow, no `cargo-dist`.
7. Dependencies are four months stale with several majors behind (toml 0.8 → 1.1, dirs 5 → 7, rand 0.9 → 0.10, nix 0.29 → 0.31, askama 0.15 → 0.16, comfy-table 7 → 8); `cargo audit` reports 0 vulnerabilities and 5 allowed warnings including RUSTSEC-2026-0386 (tokio 1.51) and RUSTSEC-2026-0097 (rand). Lock file dated 2026-06-22.
8. Product gap to be honest about: `gen-config` emits the raw `command`/`args`/`env` (`src/gen_config.rs:151-155`). Claude Code and Cursor therefore spawn their *own* server processes; the processes mcp-hub supervises are separate instances. The tool is a monitor and config generator, not a proxy. `transport = "http"` is accepted by the config (`src/config.rs:140`) but such servers are skipped in output (`gen_config.rs:143-146`) and health is stdio-only (`src/mcp/health.rs`).
9. Competitors solve the *other* half: metamcp (2.7k stars) and TBXark/mcp-proxy (741) aggregate many servers behind one SSE/streamable-HTTP endpoint with OAuth and namespacing; sparfenyuk/mcp-proxy (2.8k, the one the portfolio notes meant) bridges stdio↔HTTP; mcpm.sh (1.0k) manages client configs for 10 clients and is on Homebrew. None of them is a local process supervisor with health/restart/logs: that niche is open but small.
10. Portfolio docs: `00-MASTER-ANALYSIS.md:157` scored it difficulty 5/10, value 8/10, Phase 2 "Homebrew + binary release" (`:278`). `PORTFOLIO-VERDICT.md:50` (2026-08-28) put it in "harvest, then archive": "leave it published on crates.io, stop investing." The "34.4k LOC" there counts `.planning/`; real code is ~10.5k.

## Decision: revive minimally, time-boxed

**Recommendation: revive, but as a two-to-three-weekend experiment with a hard stop (step 8), not a reopening of the roadmap.**

Why revive: the pitch ("PM2 for MCP servers") is one sentence and lands instantly; the code works today (226/226, builds on the Mac that users have); the two things blocking any adoption: no macOS binary and dead CI: are mechanical and cost under a day; the supervisor niche is unoccupied by the aggregator projects. Letting it rest at 0.1.1 with Linux-only binaries guarantees the current outcome (0 stars, 47 downloads).

Why "minimally": the 2026-08-28 verdict is right that nothing here has met a user. The plan below spends ~20 hours to *find out* whether anyone wants a local MCP supervisor, and sets thresholds for stopping. It does not build the v2 feature list.

Does this plan agree with the portfolio docs? Partly. It agrees with `00-MASTER-ANALYSIS.md` that the deliverable is "Homebrew + binary release" and with `PORTFOLIO-VERDICT.md` that demand, not shipping, is the open question. It disagrees with the verdict's "stop investing" only in degree: a bounded ~20 h to put the tool in front of Mac users is cheaper than the months already spent, and the verdict was written when CI was presumed alive: it wasn't, so the tool has never actually been *installable* for its audience.

Evidence that would flip the decision to "let it rest":
- `cargo-dist` or hosted runners cannot produce working darwin binaries within one weekend (unlikely; `agent-memory` ships darwin from hosted runners already).
- After step 8's week: fewer than ~25 stars *and* no external issue/PR *and* the post gets no substantive comments. Then record the retro and archive with a pointer, as the verdict said.
- A vendor ships the same thing (e.g. Claude Code or Cursor adds server health/restart/logs natively). Then the niche is gone; archive.

## Depends on / unblocks

Depends on nothing; the release template is `UnityInFlow/agent-memory`'s hosted `release.yml`. Unblocks
nothing. Shared conventions (hosted runners, trusted publishing, tap by hand, VHS, README top, MCP
directories, Windows deferred): `PORTFOLIO-DECISIONS-2026-10-24.md` in the workspace root.

## First 2 hours (do before anything else)

1. Move the `include = [...]` list from `[lib]` to `[package]` (`Cargo.toml:46-55`), confirm
   `cargo package --list` has no `.planning/`, publish `mcp-server-hub` 0.1.2 by hand so the crate on
   crates.io stops shipping the planning folder.
2. Add the README "Naming" paragraph (option A) and GitHub topics (`mcp`, `model-context-protocol`,
   `process-manager`, `rust`, `claude-code`, `cursor`).
3. Nothing else until injection-scanner and agent-memory have launched; this tool is third.

**Stop rule.** Week 1 after the v0.2.0 post (step 8): fewer than 25 stars and no external issue or
PR and no substantive comment. Then write the retro, archive with a pointer, leave crates.io and the
tap installable. Nothing from "Later" before that gate.

## Launch goal and definition of done

v0.2.0 that a Mac user can install in one line and understand in 20 seconds.

- [ ] `ci.yml` + `release.yml` on GitHub-hosted runners; CI green on `main` (#3)
- [ ] GitHub Release `v0.2.0` with `aarch64-apple-darwin`, `x86_64-apple-darwin`, Linux gnu x86_64/aarch64 (+ musl best-effort), `SHA256SUMS.txt` (#3, #7)
- [ ] `cargo binstall mcp-server-hub` resolves the darwin asset in strict mode (`scripts/verify-published.sh 0.2.0`) (#7)
- [ ] `brew install unityinflow/tap/mcp-hub` works on Apple Silicon (#7)
- [ ] README: naming section + "what it does / does not do" + demo GIF (#5, #8)
- [ ] Listed in the official MCP Registry (if it accepts a supervisor; else skip) and one awesome-mcp list; one r/ClaudeAI post (#9)
- [ ] Week-1 retro with metrics and a go/no-go (#10)

## Step-by-step plan

### Step 1: CI to GitHub-hosted runners + macOS builds: #3: 4-6 h
- **What:** replace every self-hosted `runs-on` in `ci.yml` and `release.yml`; add `aarch64-apple-darwin` and `x86_64-apple-darwin` to the build matrix; fix the Cyrillic `О` at `release.yml:97` (#1).
- **Why:** nothing else can ship until CI can run (finding 5); macOS is the audience (finding 6).
- **How:** `ci.yml` → `ubuntu-latest` + `macos-14` legs running fmt/clippy/build/test. `release.yml` keeps its job chain (preflight → test → build-binaries → release → publish-crates) but on `ubuntu-latest`; darwin targets build on `macos-14` (`rustup target add x86_64-apple-darwin` cross-compiles natively; the existing host-arch-aware smoke step at `release.yml:200-212` already handles "presence-only" for the non-host arch); Linux gnu/musl stay on the pinned cargo-zigbuild path (zig 0.14.1, cargo-zigbuild 0.23.0) on `ubuntu-latest`. Extend the archive validation list (`release.yml:250-254`) and `SHA256SUMS` (`:268-273`) and the release body table. Template to copy: `UnityInFlow/agent-memory` release.yml (hosted, six-target matrix, darwin hard-required, musl `continue-on-error`).
- **Done when:** green CI on a push to `main`; a dry-run tag (`v0.2.0-rc.1`) yields the four required archives and the release job validates them.

### Step 2: Dependency refresh + test run: #4: 2-4 h
- **What:** `cargo update`, then major bumps one at a time (tokio patch, toml 1.x, dirs 7, nix 0.31, rand 0.10, comfy-table 8, dialoguer 0.12, askama/askama_web 0.16, tower-http 0.7); dedupe `thiserror` 1.x/2.x.
- **Why:** finding 7; shipping a 0.2.0 with known-unsound tokio/rand advisories is a bad first impression for a Rust audience.
- **How:** each bump followed by `cargo test` (baseline 226) + `cargo clippy -- -D warnings`. Note API changes in release notes. Add `.github/dependabot.yml` for cargo, monthly.
- **Done when:** tests ≥ 226 green, `cargo audit` 0 vulnerabilities and no tokio/rand unsound warnings, clippy and fmt clean.

### Step 3: Naming cleanup + packaging fix: #5: 1-2 h
- **What:** (a) move `include = [...]` (`Cargo.toml:46-55`) from under `[lib]` to `[package]`; (b) README "Naming" section; (c) GitHub topics.
- **Why:** (a) because the key is silently ignored, `cargo package --list` ships **155 files** including all of `.planning/**`, `.github/**`, `CLAUDE.md`, `test-web.toml`: the 0.1.1 crate on crates.io contains the whole planning folder. (b) crate ≠ repo ≠ binary confuses every first-time user (finding 4).
- **How / options for the owner:**
  - **A (recommended):** keep crate `mcp-server-hub`, repo and binary `mcp-hub`. Add under the README title: *"Repo and binary: `mcp-hub`. Crate on crates.io: `mcp-server-hub` (the `mcp-hub` crate name belongs to an unrelated project). Install with the crate name, run with the binary name."* Zero churn; the binstall contract in `Cargo.toml:23-26` is already tuned for this.
  - **B:** rename the binary to `mcp-server-hub` as well: one name everywhere, but users type it 20× a day and the tap formula/README/demo all change.
  - **C:** rename the GitHub repo to `mcp-server-hub` (GitHub redirects); keep binary and formula `mcp-hub`. Cleaner on crates.io, otherwise cosmetic.
- **Done when:** no manifest warning; `cargo package --list` has no `.planning/` paths; README section present; topics set (`mcp`, `model-context-protocol`, `process-manager`, `rust`, `claude-code`, `cursor`).

### Step 4: moved to "Later" (#6)
Codex / Gemini `gen-config` formats are a feature, and no feature ships before a user asks. #6 stays
open and gated.

### Step 5: v0.2.0 release + Homebrew formula: #7: 2-3 h (+ CI wait)
- **What:** bump to 0.2.0, tag, verify, add `Formula/mcp-hub.rb` to `UnityInFlow/homebrew-tap`.
- **Why:** first macOS-capable release; `brew install` is what Mac users expect (DIST-02, `.planning/REQUIREMENTS.md:82`, never delivered). The tap exists with `agent-memory.rb`/`prompt-vc.rb` in exactly the `on_macos`/`on_linux` + `Hardware::CPU.arm?` shape needed.
- **How:** after #3, #4, #5 merge: set `version = "0.2.0"`, write notes, `git tag v0.2.0 && git push origin v0.2.0` (preflight enforces tag == Cargo version, `release.yml:42-59`). Run `bash scripts/verify-published.sh 0.2.0` on this Mac (strict binstall). Copy `agent-memory.rb` → `mcp-hub.rb`, URLs `mcp-server-hub-<triple>-v0.2.0.tgz` (our assets are crate-named and versioned), sha256 from `SHA256SUMS.txt`, `bin.install "mcp-hub"`, test `mcp-hub --version`. Keep the tap update manual for now (agent-memory deliberately avoids org secrets in public workflows); automate later if releases become frequent.
- **Done when:** release has 4+ archives + sums; crates.io shows 0.2.0; `brew tap unityinflow/tap && brew install mcp-hub && mcp-hub --version` and `brew test mcp-hub` pass on Apple Silicon.

### Step 6: README top rewrite + 20-second demo: #8: 2-3 h
- **What:** replace README lines 1-16 with the block below; add `docs/demo.gif`.
- **Why:** the current README buries the tool under install tables and never says what it is *not* (finding 8).
- **How: proposed top block (replace lines 1-16, then the Naming section from step 3, then the existing Installation section):**

```markdown
# mcp-hub

**PM2 for MCP servers.** One TOML file, one command, one log stream: for every
MCP server you run locally.

    brew install unityinflow/tap/mcp-hub     # macOS / Linux (or: cargo binstall mcp-server-hub)
    mcp-hub init                             # add a server interactively -> ./mcp-hub.toml
    mcp-hub start --daemon                   # supervise all servers, auto-restart with backoff
    mcp-hub status                           # name, state, health, PID, uptime, restarts
    mcp-hub logs -f                          # all servers interleaved, docker-compose style
    mcp-hub gen-config --format claude       # paste-ready mcpServers block for Claude Code or Cursor

![20-second demo](docs/demo.gif)

**What it does:** starts and supervises your stdio MCP servers, pings them for health,
restarts crashed ones (1s -> 2s -> 4s -> 60s), streams their logs, shows tools per server
in a web UI at http://localhost:3456, and generates client config blocks.

**What it does not do (yet):** it is not a proxy or aggregator. Clients still spawn their
own server processes from the generated config; mcp-hub runs and watches a parallel set.
If you want one endpoint that fans out to many servers, look at metamcp or mcp-proxy.
Remote (SSE / streamable-HTTP) servers are not health-checked.
```

- Demo script (20 s, VHS tape at `docs/demo/demo.tape`, offline bash servers as in `test-web.toml`): 0-3 s `mcp-hub init` adding `echo-server`; 3-6 s `mcp-hub start --daemon`; 6-9 s `mcp-hub status` table; 9-12 s `kill <pid>`; 12-15 s `mcp-hub status` shows `restarts: 1`; 15-18 s `mcp-hub logs -n 5`; 18-20 s browser at `localhost:3456`.
- **Done when:** block in place, GIF renders on GitHub, limits stated.

### Step 7: Listings + one post: #9: 2-3 h
- **What:** PRs to `punkpeye/awesome-mcp-servers` and `wong2/awesome-mcp-servers` (tooling sections); `mcpservers.org`; one r/ClaudeAI post cross-posted to r/mcp and r/rust. PulseMCP takes no submissions (it ingests the official registry); the official registry lists servers, so a supervisor may not qualify: check once, do not force it.
- **Why:** 0 inbound links; discovery is the bottleneck.
- **How: post draft:** Title: *"I made a PM2 for MCP servers (Rust, single binary): start/stop/health/auto-restart/logs/web UI, generates Claude Code and Cursor configs."* Body: the problem (five-plus stdio servers, each client spawns its own, nothing tells you one died at 2 am); the GIF; `brew install unityinflow/tap/mcp-hub`; honest limits (not a proxy, macOS + Linux only); the ask: *"what would make you use this instead of nothing?"* Reply to every comment within 24 h.
- **Done when:** one awesome-list PR merged or open without blocking feedback; post published and linked from #2.

### Step 8: First week: #10: ~3 h spread
- **What:** triage, patch-only fixes (`v0.2.1`), metrics, retro, go/no-go.
- **Why:** this is the experiment the verdict asked for; decide on evidence.
- **How:** day 0 release + listings + post; days 1-2 answer everything within 24 h; days 3-5 file requests as issues, implement nothing beyond one-liners; day 7 write `docs/reviews/2026-11-xx-week1-retro.md` with stars, clone traffic (`gh api repos/UnityInFlow/mcp-hub/traffic/clones`), crate downloads (baseline 47), release-asset downloads (`gh release view v0.2.0 --json assets`), external issues/PRs, post engagement.
- **Done when:** retro committed and one of (a) continue to v0.3 `attach`, (b) maintenance only, (c) archive with pointer is recorded in #2. Suggested thresholds: ≥ 25 stars or ≥ 3 external issues/PRs → (a); otherwise (b).

## Decisions the owner must make

Defaults are recorded in `PORTFOLIO-DECISIONS-2026-10-24.md`, "Owner decisions"; tap automation and
Windows are shared decisions S3 and S5 there.

- **D1: Release tooling:** hand-written `release.yml` (default; binstall contract already aligned, ~1 h of edits) vs `cargo-dist` (generates workflow, installers and the Homebrew formula, but re-plumbs asset naming and the crate≠binary contract). Needed before #3.
- **D2: Naming:** option A / B / C in step 3. Default A. Needed before #5 and #8.
- **D3: Time box:** accept step 8's thresholds, or set your own. Needed before #9 so the post's promises match the commitment.

## Later, only if demand (gated on step 8 saying "continue")

- **Codex / Gemini CLI `gen-config` formats** (#6, 2-3 h): `--format codex|gemini` (optionally `claude-desktop`, `windsurf`, `vscode`) as a clap `ValueEnum`; renderers beside `render_claude_config` (`src/gen_config.rs:46-64`); `gemini` writes JSON `mcpServers` for `~/.gemini/settings.json`, `codex` writes TOML `[mcp_servers.<name>]` for `~/.codex/config.toml`; verify key names against current docs; golden tests in `tests/gen_config_test.rs`.
- **`mcp-hub attach <name>`** (v0.3 candidate): a stdio shim that clients run instead of the raw command, forwarding over the daemon's Unix socket to the *supervised* process. The dispatcher already owns child stdin/stdout (`src/mcp/dispatcher.rs:79`) and the control protocol exists (`src/control.rs:25`); this turns the tool from "monitor a parallel copy" into "the one process the client actually uses" and is the honest answer to finding 8. Scope it before starting; it is a week, not a weekend.
- HTTP / streamable-HTTP health checks for remote servers (TRANS-01, `.planning/REQUIREMENTS.md:88`).
- Windsurf / VS Code / Claude Desktop formats if not done in step 4 (ECO-01/02).
- Log persistence with rotation (ADV-04): the most common PM2 feature still missing.
- Not planned: aggregation/namespacing/OAuth. That is metamcp's and mcp-proxy's territory; competing there means a different product.

## Issue index

| # | Title | Step | Effort |
|---|---|---|---|
| [#2](https://github.com/UnityInFlow/mcp-hub/issues/2) | Revive and launch v0.2.0: macOS builds, hosted CI, listings (2026-10-24 plan) | tracking | n/a |
| [#3](https://github.com/UnityInFlow/mcp-hub/issues/3) | Move CI/release to GitHub-hosted runners and add macOS arm64 + x86_64 builds | 1 | 4-6 h |
| [#4](https://github.com/UnityInFlow/mcp-hub/issues/4) | Dependency refresh + cargo audit clean | 2 | 2-4 h |
| [#5](https://github.com/UnityInFlow/mcp-hub/issues/5) | Naming cleanup and fix misplaced Cargo `include` list | 3 | 1-2 h |
| [#6](https://github.com/UnityInFlow/mcp-hub/issues/6) | gen-config formats for Codex CLI and Gemini CLI | later, gated | 2-3 h |
| [#7](https://github.com/UnityInFlow/mcp-hub/issues/7) | Release v0.2.0 and add Formula/mcp-hub.rb to homebrew-tap | 5 | 2-3 h |
| [#8](https://github.com/UnityInFlow/mcp-hub/issues/8) | README top rewrite + 20-second demo GIF | 6 | 2-3 h |
| [#9](https://github.com/UnityInFlow/mcp-hub/issues/9) | Listings and r/ClaudeAI launch post | 7 | 2-3 h |
| [#10](https://github.com/UnityInFlow/mcp-hub/issues/10) | First week after launch: triage, metrics, go/no-go retro | 8 | ~3 h |
| [#1](https://github.com/UnityInFlow/mcp-hub/issues/1) | Cyrillic О homoglyph in release.yml (pre-existing) | folded into #3 | n/a |

Total: ~16-24 h. Critical path: #3 → #7; #4, #5, #8 can run in parallel with #3. #6 is gated.
