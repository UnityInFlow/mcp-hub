# mcp-hub: Fable analysis (2026-10-25)

Author: Claude Fable 5.1 (model id claude-fable-5-1), working with the owner in Claude Code.
Status: opinion document. A second opinion from Claude Opus is requested; the prompt for it is in the appendix.
Inputs: repo at 0c60172, the revive plan `docs/reviews/2026-10-24-launch-plan.md`, `docs/TODO.md`, web research on 2026-10-25 (links inline).

## Summary in ten lines

1. Verdict: pivot. Ship v0.2.0 only if it includes an exclusive `mcp-hub attach <name>` shim, so the process the daemon supervises is the process Claude Code and Cursor actually talk to. Shipping the current monitor with macOS binaries, as the plan proposes, spends 20 hours on a tool whose own README top block admits it does not solve the problem it names.
2. If the owner cannot give about 30 hours inside six weeks, do the two-hour packaging fix (crate ships `.planning/`) and let the tool rest. The middle option is the worst one.
3. Killer risk: the pain is real and loud in 2026 (orphaned MCP processes, one process per client window, servers dying silently), but today mcp-hub adds a parallel copy of every server and fixes none of the four complaints. The honest paragraph is in section 3.
4. Second risk: VS Code already has per-server start, stop, restart and Show Output; Claude Code has `/mcp reconnect all` and `claude mcp list` health. Monitoring alone is being absorbed by clients. Sharing one process across clients is not, and no client can do it from inside itself.
5. Differentiator that survives: one supervised stdio process per server, shared by every client through a stdio shim, with restart that the client never sees. Prior art exists as a library (`rmcp-mux`) and inside aggregators (1mcp) but not as a single-binary, no-HTTP, TOML-first tool for Mac users.
6. Code quality is good for its size (226 tests, clippy clean, no production `unwrap`), but section 5 lists eight behaviours that contradict the README, two of which the plan's demo script would expose on camera.
7. Health does not drive restart. `Failed` is a label; only process exit restarts a server (`src/supervisor.rs:406-466`, `src/mcp/health.rs:132-134`).
8. Naming: `ravitemer/mcp-hub` (524 stars, npm `mcp-hub`) does strictly more under the same name. Option A in the plan keeps the collision; I lean to option B.
9. Stop rule: keep the plan's thresholds (25 stars or 3 external issues or PRs by day 30, S7), but measure them after an `attach` launch, not after a monitor launch, or the experiment answers the wrong question.
10. Budget: 12 h mechanical (hosted CI, darwin, packaging, README truth) plus 16 to 18 h exclusive attach, three weekends, docs capped at 20 percent.

## Where it stands

Verified on this Mac on 2026-10-25 against `main` at `0c60172`.

| Fact | Value | How checked |
|---|---|---|
| Tests | 226 passed, 0 failed, 20 binaries | `cargo test`, summed `test result` lines |
| Lint | clean | `cargo clippy --all-targets -- -D warnings` |
| Audit | 0 vulnerabilities, 5 warnings | `cargo audit` 0.1.10: basic-toml unmaintained (RUSTSEC-2026-0374), anyhow 1.0.102 unsound (RUSTSEC-2026-0190), rand 0.8.5 and 0.9.2 both in tree, unsound (RUSTSEC-2026-0097), tokio 1.51.0 unsound (RUSTSEC-2026-0386). The plan lists tokio and rand only. |
| Size | 5,414 LOC `src/`, 4,457 `tests/`, 235 locked packages | `wc -l`, `Cargo.lock` |
| Packaging bug | `cargo package --list` = 157 files, 86 under `.planning/` | `include` sits under `[lib]` at `Cargo.toml:46-55`, so it is ignored |
| CI | dead since 2026-06-22 | `gh run list`: CI runs `cancelled`, last Release success 2026-06-23 13:00 UTC; `ci.yml:13` and five `runs-on` in `release.yml` name `arc-runner-unityinflow` or `orangepi` |
| Releases | v0.1.0 (2026-04-09), v0.1.1 (2026-06-23), Linux only | `gh release list`; v0.1.1 asset downloads: 1 tarball, 2 SHA256SUMS |
| crates.io | `mcp-server-hub` 0.1.1, 47 downloads total, 4 in 90 days | crates.io API |
| GitHub | 0 stars, 0 forks, 10 issues all by the owner, 21 clones / 17 unique in 14 days | `gh api` |
| Naming triple | repo `mcp-hub`, binary `mcp-hub`, crate `mcp-server-hub`; crate `mcp-hub` is gleb-chipiga's aggregator (0.3.0, 582 downloads, 0 GitHub stars); npm `mcp-hub` is ravitemer's | crates.io API, `gh api` |
| Commit mix | 180 commits, 90 with a `docs` prefix (50 percent), 75 `feat`, `fix` or `test` | `git log --format=%s` |
| Toolchain | rustc 1.94.1 locally, no `rust-toolchain.toml`, no `rust-version` in `Cargo.toml` | repo tree |
| Governance drift | `CLAUDE.md:40` demands Windows binaries, `CLAUDE.md:88` says "Never use `ubuntu-latest`"; portfolio S1 and S5 say the opposite | `CLAUDE.md`, `PORTFOLIO-DECISIONS-2026-10-24.md` |

## Market and alternatives

Stars and dates from the GitHub API on 2026-10-25.

| Project | Stars | Last release | Supervises stdio children | Health / restart | Proxies to clients | Licence | Source |
|---|---|---|---|---|---|---|---|
| metatool-ai/metamcp | 2,724 | v2.4.22, 2025-12-19 (pushed 2026-06-22) | yes, inside its container | unverified | yes, SSE / streamable HTTP, OAuth | MIT | https://github.com/metatool-ai/metamcp |
| sparfenyuk/mcp-proxy | 2,805 | v0.13.0, 2026-10-10 | one child per bridge | no | stdio to HTTP bridge | MIT | https://github.com/sparfenyuk/mcp-proxy |
| TBXark/mcp-proxy | 741 | v1.1.0, 2026-09-15 | yes | unverified | yes, aggregates | MIT | https://github.com/TBXark/mcp-proxy |
| docker/mcp-gateway | 1,615 | no GitHub release object | one container per catalog server | not documented | stdio default, `sse` and `streaming` serve many clients; examples for claude-code and cursor; works without Docker Desktop | MIT | https://github.com/docker/mcp-gateway and https://docs.docker.com/ai/mcp-catalog-and-toolkit/mcp-gateway/ |
| 1mcp-app/agent | 524 | v0.39.4, 2026-10-24 | yes ("owned stdio backends") | opt-in automatic recovery, status visibility, operator restart | yes, one HTTP endpoint for Cursor and Claude Code | Apache-2.0 | https://github.com/1mcp-app/agent |
| mcpjungle/mcpjungle | 1,309 | 0.4.6, 2026-08-02 | yes | not stated | yes, streamable HTTP `/mcp` | MPL-2.0 | https://github.com/mcpjungle/mcpjungle |
| pathintegral-institute/mcpm.sh | 1,014 | v2.15.0, 2026-05-22 | no | no | no; client config manager, on Homebrew | MIT | https://github.com/pathintegral-institute/mcpm.sh |
| ravitemer/mcp-hub (Node, npm `mcp-hub`) | 524 | v4.2.1, 2025-08-22 | yes, start/stop/enable | "health monitoring and automatic recovery" | yes, `localhost:37373/mcp` for all clients | MIT | https://github.com/ravitemer/mcp-hub |
| gleb-chipiga/mcp-hub (crate `mcp-hub`) | 0 | v0.3.0, 2026-07-29 | yes | unverified | yes, aggregator | MIT | https://github.com/gleb-chipiga/mcp-hub |
| smithery-ai/cli | 849 | v1.2.0, 2026-05-31 | no | no | no; installer | AGPL-3.0 | https://github.com/smithery-ai/cli |
| Bigsy/mcpmu (Go, Homebrew tap) | 19 | pushed 2026-09-25 | yes, lazy or eager | `doctor` and `status`, no auto restart stated | yes, `serve --stdio` or `--http` | MIT | https://github.com/Bigsy/mcpmu |
| Loctree/rmcp-mux (Rust library, 0.4.0, 2026-06-08) | n/a, 184 crate downloads | 2026-06-08 | one child, many clients over Unix sockets | exponential backoff restart | stdio shim, cached initialize, id rewriting | unverified | https://docs.rs/rmcp-mux |
| jschoemaker/mcpgo (npm) | 0 | 2026-05-20 | wraps one stdio server, respawns child so Claude Code stays connected | restart on demand | stdio passthrough | none | https://github.com/jschoemaker/mcpgo |

Native client features in 2026:

- Claude Code (v2.1.310 on 2026-10-23): `/mcp` shows status and a per-server Reconnect; `/mcp reconnect all` since v2.1.284; `claude mcp list` prints a health status per server; remote servers get five reconnect attempts with backoff; a stdio server that dies "is marked failed" and needs manual Reconnect; no log viewer beyond `--debug-file`. https://code.claude.com/docs/en/mcp. Feature requests for `claude mcp restart` were closed as resolved in January and March 2026: https://claudeissues.com/issue/17690-feature-request-add-per-mcp-server-restart-command-and-health-monitoring and https://claudeissues.com/issue/38138-feature-request-claude-mcp-restart-command-for-reconnecting-hanging-mcp-servers.
- VS Code: Start, Stop, Restart and Show Output per server from the Extensions view and `mcp.json` lenses. https://code.visualstudio.com/docs/copilot/guides/mcp-developer-guide
- Cursor: "MCP Logs" output channel and a Reload button; staff say a full restart is still the dependable path. https://forum.cursor.com/t/cant-properly-reload-locally-running-mcp-server/164434
- Codex CLI: `codex mcp add|list|get|remove|login|logout`, `/mcp` in the TUI, no restart or logs command. https://developers.openai.com/codex/mcp and https://github.com/openai/codex/issues/20195
- Windsurf: status icon, restart means restarting the app. https://fast.io/resources/windsurf-mcp-setup-guide.md (community source, unverified against vendor docs)

What users complain about:

- Orphans: ~80 orphaned MCP processes and ~13 GB swap after three days with 10 servers (https://claudeissues.com/issue/26658-mcp-server-processes-are-never-cleaned-up-when-sessions-end-orphan-accumulation); npx grandchildren reparented to PID 1, open since July 2026 (https://claudeissues.com/issue/76306-stdio-mcp-servers-spawned-via-launcher-commands-bun-run-npx-leave-orphaned-grand); macOS PPID=1 accumulation (https://claudeissues.com/issue/33947-bug-mcp-server-and-subagent-processes-not-cleaned-up-on-session-end-orphan-accum).
- Duplicates per client or window: Cursor staff confirmed duplicate connections (https://forum.cursor.com/t/windows-mcp-json-major-bug-with-non-stdio-servers/145414); Claude Desktop 6 processes for 3 servers at 92 percent RAM (https://claudeissues.com/issue/43597-bug-claude-desktop-v1-569-0-mcp-servers-spawn-duplicate-processes-on-every-resta); concurrent Claude Code sessions multiplying servers to a kernel panic (https://claudeissues.com/issue/45880-bug-multiple-concurrent-sessions-multiply-mcp-server-processes-without-memory-li); ~10 GB idle RAM per workspace in Antigravity (https://discuss.ai.google.dev/t/bug-mcp-servers-spawn-per-workspace-causes-process-explosion-and-10-gb-idle-ram/129054).
- Silent death: the Claude Code docs above; `mcpgo` exists because "Claude Code marks a crashed server as failed with no automatic recovery".
- Config drift across clients: what mcpm.sh and 1mcp presets sell.

The honest paragraph. Today mcp-hub supervises a second set of processes that no client talks to (`src/gen_config.rs:151-155` emits the raw command, so Claude Code spawns its own copy). Against the four complaints: it adds one more process per server (duplicates get worse), it cannot reap a client's orphans, the server that dies under Claude Code is not the one it restarts, and its config generator does not expand `${VAR}` (section 5) and writes resolved secrets into the client file. So the thing it fixes today is "I want a dashboard and one TOML file", which mcpm.sh and VS Code already give their users. Docker MCP Gateway owns the containerised, catalog-driven version of "one process per server, many clients" and ships with Docker Desktop, but it needs Docker, a catalog entry or an OCI image, and runs over its own transport; it does not supervise an arbitrary `npx` command on a laptop without containers. 1mcp owns the HTTP aggregator version. The gap that is still open in October 2026 is: single binary, no Docker, no HTTP, keep my existing stdio configs, and make the client use the one supervised process. That is `attach`, and it is also the only version of this tool whose demo is not "restart a process nobody uses".

## Design

Architecture as built: a tokio daemon forked before the runtime (`src/daemon.rs:162-191`), one supervisor task per server (`src/supervisor.rs:207-521`), a one-request-one-response Unix socket protocol (`src/control.rs:112-114`, `:168-193`), an axum web UI bound to 127.0.0.1 (`src/web/mod.rs:55`), and offline config generation (`src/gen_config.rs:117-172`). The split is sound and small. Three design choices cap it:

- Health model: `ping` only, stdio only. Spec-wise this is right (ping may precede `initialize`: https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle, and the receiver MUST answer: https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/ping). But an error reply counts as a miss (`src/mcp/health.rs:34-39`), introspection waits for the first `Healthy` (`src/supervisor.rs:367`), so a server that rejects `ping` is never introspected and reads `failed (7 missed)` while it works fine for real clients. And nothing acts on `Failed`: the supervisor restarts only on exit (`src/supervisor.rs:408`). "Health monitoring with auto-restart" (`CLAUDE.md:7`) is two unconnected features.
- Backoff: `BackoffConfig::default()` is hard-wired (`src/supervisor.rs:215`); `max_retries` and `restart_delay` are parsed (`src/config.rs:70-76`) and never read. Defaults are 1 s base, 60 s cap, 30 percent jitter, 10 attempts, 60 s stable window (`src/types.rs:60-66`). The policy itself is fine.
- Logs: per-server ring buffer of 10,000 lines (`src/main.rs:77`, `src/logs.rs:49-55`) plus a 1,024-slot broadcast (`src/logs.rs:104`). Bounded, in memory only, lost on daemon exit, and buffers are created once at startup, so a server added by `reload` has no buffer (`src/logs.rs:120-122`). Disk persistence is the most asked-for PM2 feature and the plan rightly defers it.
- Config: flat TOML with `transport` accepted but meaningless (`http` servers are still spawned and pinged over stdio, `src/supervisor.rs:583-584`, and skipped only by gen-config at `src/gen_config.rs:143-146`). Protocol version pinned to `2024-11-05` (`src/mcp/protocol.rs:185`); Cursor users already report servers that reject legacy `initialize` (https://forum.cursor.com/t/mcp-client-cannot-connect-to-modern-only-2026-07-28-streamable-http-servers-legacy-initialize-rejected/172536).

`attach`, concretely:

- Protocol: `mcp-hub attach <name>` connects to the daemon socket, sends `{"cmd":"attach","name":...}`, and the connection becomes a bidirectional newline-delimited JSON pipe: shim stdin to child stdin, child stdout to shim stdout. This breaks the one-request-one-response contract in `src/control.rs:168-193`, so `Attach` needs its own branch that hands the `UnixStream` to a session task instead of returning.
- Where it hooks: the dispatcher already owns child stdout and routes by `id` (`src/mcp/dispatcher.rs:79-107`) and serialises stdin writes (`:144-155`). A session object must sit between the supervisor and the child so it survives a restart: today `pending`, `stdin_shared` and `id_alloc` are created per spawn (`src/supervisor.rs:314-317`) and die with the child.
- Exclusive mode (one client per server) needs: id rewriting between client ids and child ids (client ids may be strings; `JsonRpcResponse.id` is `u64` at `src/mcp/protocol.rs:62`, so the response type widens to `serde_json::Value`), passthrough of notifications both ways, replay of the client's own `initialize` after a daemon-side restart, and a rule for second attachers (JSON-RPC error, or queue). Estimate 12 to 16 h including tests with the fixture servers in `tests/fixtures`.
- Multi-client fan-in adds: cache the first `initialize` result and answer later clients from it (what `rmcp-mux` does), fan out `notifications/*/list_changed` to all clients, route server-to-client requests (`sampling`, `elicitation`, `roots/list`) to the originating client or refuse them in the advertised capabilities, per-client `resources/subscribe` bookkeeping, and an idle policy when the last client detaches. Estimate another 18 to 24 h. Pitfalls documented by others: different clients negotiating different protocol versions against one cached handshake (https://github.com/punkpeye/mcp-proxy), id collisions (https://docs.rs/rmcp-mux).
- Minimal design I propose for v0.2.0: exclusive attach plus transparent restart, `gen-config` emitting `{"command":"mcp-hub","args":["attach","github"]}` with no env (secrets stay in `mcp-hub.toml` and `env_file` on the daemon side), and the daemon idle-stopping a server 30 minutes after its client detaches (matching Claude Code's stdio idle window). That fixes silent death, orphans and config drift for one client, and duplicates across sequential sessions. Multi-client mux is v0.3 if anyone asks for it; the second attacher gets a clear error that names the issue to upvote.
- Should it stay a monitor? Only if the owner wants a 20 h experiment that answers "do people want a dashboard", a question VS Code and mcpm.sh have already answered with "they have one".

## Improve

Code risks, highest first:

| # | Risk | Where | Note |
|---|---|---|---|
| 1 | Daemon shutdown can exit without stopping children | `src/main.rs:186-199` | `socket_task.await` waits for the accept loop only; per-connection tasks (`src/control.rs:132-137`) hold `Arc<DaemonState>`; `Arc::try_unwrap` at `:194` then fails and `stop_all_servers` at `:199` never runs. The `Stop` handler itself holds the Arc while writing its reply (`src/control.rs:185-191`), so the window is on the normal `mcp-hub stop` path. Children are in their own process groups (`src/supervisor.rs:60`) with `kill_on_drop(false)` (`:56`), so they become exactly the orphans the tool exists to prevent. |
| 2 | Orphans on daemon crash or SIGKILL | `src/supervisor.rs:56-60`, `src/daemon.rs` | No PDEATHSIG, no record of child PIDs on disk, no adoption on restart; a new daemon spawns a second set. Write `<name>.pid` files next to the socket and reap or adopt at startup. |
| 3 | XSS through server stderr | `src/web/sse.rs:54-57` | `line.message` and `line.server` interpolated into HTML without escaping; a server log line containing `<img src=x onerror=...>` executes in the operator's browser. Escape or send plain text and render client-side. |
| 4 | README promises `${VAR}` expansion | `README.md` Configuration section; `src/config.rs:160-181` | No expansion code exists (`grep -rn 'expand' src/` is empty). `${GITHUB_TOKEN}` is passed literally to the child and to the generated client config. |
| 5 | Secrets copied into client configs | `src/gen_config.rs:148-155` | `env_file` values are resolved and printed; `attach` removes the need. |
| 6 | `logs -f` is a dead flag | `src/cli.rs:79-82`, `src/main.rs:320-348` | `follow` never read; the plan's README block advertises it. Needs a streaming socket command or a documented removal. |
| 7 | `status` prints raw JSON in daemon mode | `src/main.rs:304-307` | The table exists only for the foreground `status` stdin command (`:580-582`). The plan's demo script step "mcp-hub status table" will show JSON. |
| 8 | Reload-added servers lose logs | `src/logs.rs:98-107`, `:120-122` | Buffers fixed at startup; `push` for an unknown name is a silent no-op. |
| 9 | Health `Failed` never restarts | `src/mcp/health.rs:132-134` | Either restart after N misses (opt-in) or stop calling it monitoring with auto-restart. |
| 10 | Unused config keys | `src/config.rs:70-76` vs `src/supervisor.rs:215` | `max_retries`, `restart_delay` ignored; README documents a nested `[servers.x.restart]` and `[servers.x.health]` schema that does not exist (`KNOWN_SERVER_FIELDS` is flat, `src/config.rs:8-18`). |
| 11 | Socket mode | `src/control.rs:122` | Bound with the process umask, no `chmod 0600`; low risk on a single-user Mac, cheap to fix. |
| 12 | Reload blocks status | `src/main.rs:418-426` | Holds the handles lock through `apply_config_diff`, up to 5 s per stopped server (`src/supervisor.rs:139`); `status` and the web UI stall. |

Missing tests: none for the shutdown race (1), none that a `reload`-added server appears in `logs` (8), none for `${VAR}` (4, a test would have caught the README), none for HTML escaping (3), none for `logs -f` (6), none asserting what `Failed` health does (9). Integration tests use real fixture binaries, which is good; a slow fixture that answers `ping` with an error would cover the introspection dead end in one test.

Dependencies and audit: lock file from 2026-06-22 (tokio 1.51.0, toml 0.8.23). The two `rand` versions mean a transitive crate still pulls 0.8; the unsound advisories are conditional (custom logger, `AsyncFd` misuse) and none applies to this code path as far as I can see, so I would `cargo update` within semver before launch and leave the major bumps (toml 1.x, dirs 7, nix 0.31, askama 0.16) for after the week-1 gate. The jitter at `src/supervisor.rs:166-176` is the only `rand` use; a 20-line xorshift drops the dependency and the advisory.

## Build

Release pipeline review (`.github/workflows/release.yml`): the job chain preflight, test, build, release, publish is right, the zigbuild pins (`:20-21`) are right, and the binstall contract (`Cargo.toml:23-26`) is already tuned for crate not equal binary. Problems: every `runs-on` is a self-hosted label (`:32`, `:74`, `:119`, `:233`, `:329`); actions are tag-pinned (`actions/checkout@v5`, `softprops/action-gh-release@v3`) while agent-memory pins SHAs with version comments and adds `actions/attest-build-provenance` (its `release.yml:91-311`); crates.io publishes with a long-lived `CARGO_REGISTRY_TOKEN` (`:64`, `:340`) against portfolio S2 (trusted publishing); the tag filter `v*` (`:14`) would publish a milestone tag, the exact bug agent-memory documents in its header; the Cyrillic homoglyph at `:97` stands.

Hand-written workflow vs cargo-dist: cargo-dist is alive (v0.33.1 on 2026-10-21, 2,142 stars, https://github.com/axodotdev/cargo-dist) and would generate installers and the tap formula, but it re-plumbs asset names and the crate-binary split, which the binstall metadata and `scripts/verify-published.sh` already encode. Keep the hand-written file (D1 default) and copy agent-memory's: hosted `ubuntu-latest` for zigbuild Linux legs, `macos-14` for both darwin triples (native arm64 plus `rustup target add x86_64-apple-darwin`), darwin hard-required, musl `continue-on-error`, SHA-pinned actions, attestation, exact-semver tag filter. cargo-binstall is at v1.26.1 (2026-10-24, 2,940 stars, https://github.com/cargo-bins/cargo-binstall); the metadata is worth keeping.

Homebrew: core requires 30 forks, 30 watchers or 75 stars, and 90 forks, 90 watchers or 225 stars for a self-submission; "software that does not meet the official criteria can generally be maintained in a third-party tap" (https://docs.brew.sh/Package-Acceptance-Policy). The tap is the only path for the next year. Formula shape: copy `agent-memory.rb`, assets `mcp-server-hub-<triple>-v0.2.0.tgz`, `bin.install "mcp-hub"`.

Add: `deny.toml` with `cargo-deny` (advisories, licences, duplicate `rand`), `rust-version = "1.85"` or whatever `cargo msrv` finds, `.github/dependabot.yml` monthly for cargo and github-actions, a CI leg on `macos-14` so the daemon fork path is tested where users run it, and `cargo package --list | grep -c planning` as a CI assertion equal to 0.

## Develop

Process for one developer and one agent:

- Time box: weekend 1 is mechanical (hosted CI, darwin, packaging fix, README truth, `status` table in daemon mode, dead `logs -f`, XSS escape): about 12 h. Weekend 2 is exclusive `attach` with its tests: about 14 h. Weekend 3 is release, tap, demo, post, and the first days of replies: about 6 h. If weekend 2 slips past six weeks, stop and archive; do not ship weekend 1 alone.
- Fix the `CLAUDE.md` contradiction in the first commit of weekend 1: replace the "CI / Self-Hosted Runners" section with one line pointing at S1, drop Windows from `:40` and `:59` per S5, delete the GSD scaffolding instructions that describe a project not yet started (`:62-74`), and remove the GSD enforcement block so a `docs/reviews` edit does not require a planning ceremony. Keep the "no unwrap, no JS framework" rules; they are being followed.
- Docs cap: 90 of 180 commits are docs. Rule for the revive: no commit touches `.planning/`; one review file per date in `docs/reviews/`; the README and the demo tape are the only docs deliverables; count at the week-1 retro and report the percentage.
- Issue hygiene: #2 to #10 are the owner talking to himself. Before the post, retitle so an outsider can read them, add `good first issue` to the Codex and Gemini formats (#6) and to "multi-client attach", and open one issue per section-5 row 1 to 9 so the retro can count closed bugs rather than vibes.
- Agent use: the agent writes the attach session code and the fixture tests; the owner reviews the shutdown path and the id-rewriting by hand, because those are the two places where a plausible-looking patch loses children or misroutes a response.

## Deliver

Launch sequence relative to v0.2.0:

1. T minus 14 days: packaging fix, `mcp-server-hub` 0.1.2 by hand (the First 2 hours), topics (done), README naming paragraph.
2. T minus 7: hosted CI green, `v0.2.0-rc.1` dry-run tag producing six archives.
3. T minus 2: `attach` merged, demo tape recorded, README top rewritten, formula drafted locally.
4. T: tag `v0.2.0`, verify binstall strict, push the tap formula, post on r/ClaudeAI 14:00 to 16:00 CET on a Tuesday to Thursday (portfolio convention), cross-post to r/mcp the same hour and r/rust the next day.
5. T plus 1 to 7: reply within 24 h, `v0.2.1` for one-liners only, awesome-mcp-servers PR under `### Aggregators` (the list has no tooling section; metamcp, 1mcp and MCPJungle sit there, https://raw.githubusercontent.com/punkpeye/awesome-mcp-servers/main/README.md). Skip the official MCP Registry: `server.json` describes packages or remotes a client connects to, and a supervisor is neither (https://github.com/modelcontextprotocol/registry; eligibility for non-servers unverified in the official text, treat as no).
6. T plus 7: retro with stars, clones, crate and asset downloads, external issues, and the number of `attach` installs if the shim reports a version on first run (opt-in, documented).

README top block critique: the plan's block is good in shape and wrong in three facts. `mcp-hub logs -f` does not follow; "pings them for health, restarts crashed ones" reads as health-driven restart; and `mcp-hub status` in daemon mode prints JSON. Replace the command list with `init`, `start --daemon`, `attach <name>` (shown inside a `.mcp.json` snippet), `status`, `logs`, `gen-config --format claude`; replace the "does not do" paragraph with: "It is not an aggregator and adds no HTTP endpoint. Every client still sees one plain stdio server per entry; mcp-hub just makes sure it is the same process, alive, and logged. One client per server in 0.2; many clients per server is issue #N."

Demo tape critique: the plan's 20 s script kills a PID and shows `restarts: 1`. That demonstrates restarting a process nobody uses. Record instead: Claude Code with `/mcp` showing a server connected via `mcp-hub attach`; `kill -9` the server from a second pane; `mcp-hub status` shows `restarts: 1`; back in Claude Code, call a tool from that server and it answers; end on `mcp-hub logs` showing the crash line. 25 s, one message, nothing to argue with.

First-week routine: morning and evening check of GitHub notifications and the three posts, 20 minutes each; a running `docs/reviews/2026-11-xx-week1-retro.md` that is appended, not written on day 7; no feature work. "Continue" metric: 25 stars or 3 external issues or PRs by day 30 (S7), plus one qualitative gate the plan lacks: at least one person says they replaced their raw `npx` entry with `attach`. Without that sentence the stars are curiosity, not use.

r/ClaudeAI post, ready to paste (title plus body, 221 words):

Title: I made a supervisor for local MCP servers: one process per server, shared by Claude Code and Cursor, auto-restart, unified logs (Rust, single binary)

Body:

If you run five or more stdio MCP servers, you know the pattern: every Claude Code window spawns its own copy, orphaned `npx` processes pile up (there are GitHub issues with 80 orphans and 13 GB of swap), and when a server dies you find out when a tool call fails.

mcp-hub is a small Rust daemon that starts your servers once from one TOML file, pings them, restarts crashed ones with backoff, and keeps their logs in one stream. The new part in 0.2.0 is `mcp-hub attach <name>`: you put that in your `.mcp.json` instead of the raw command, and Claude Code talks to the supervised process. If the server crashes, the daemon restarts it and the client never notices.

Install: `brew install unityinflow/tap/mcp-hub` (macOS and Linux) or `cargo binstall mcp-server-hub`.

Honest limits: one client per server in this release (a second client gets a clear error; multi-client is the next issue), no HTTP aggregation, no Windows yet, logs are in memory. If you want one endpoint fanning out to many servers, look at 1mcp or metamcp instead.

20-second demo in the README: kill the server, Claude Code keeps working.

What would make you use this instead of nothing? I will answer every comment.

## Fable's opinion

1. The plan launches the wrong product first; the monitor's own README has to explain that clients ignore it, and that paragraph is the launch's epitaph.
2. `attach` is a two-weekend feature, not "a week"; exclusive mode needs id rewriting and a session that outlives the child, and nothing else, so the plan's estimate is inflated by the multi-client case it should defer.
3. The killer risk is not that Claude Code adds restart (it added `/mcp reconnect all` and still cannot share a process between windows); it is that Docker's gateway becomes the default for the Docker-using half of the audience, so the pitch must say "no Docker, no HTTP, your existing configs".
4. Health should either restart or be renamed to "status"; a `failed` label that triggers nothing will be the first bug report.
5. Option B for naming (binary `mcp-server-hub`) is better than the plan's A once `attach` exists, because the binary name is typed once into a config file rather than twenty times a day, and `ravitemer/mcp-hub` on npm will outrank this repo for years.
6. Do `cargo update` within semver and skip the major bumps before launch; the audit warnings are conditional unsoundness, not vulnerabilities, and four hours of API churn buys no user.
7. The shutdown race in `src/main.rs:194` is a release blocker because it produces the exact failure the tool exists to prevent.
8. Drop `transport = "http"` from the config until it means something; a key that spawns and pings a process over stdio while saying `http` is a support thread waiting to happen.
9. Keep the stop rule but add the qualitative gate; 25 stars from a Reddit post is a weak signal, one stranger changing their `.mcp.json` is a strong one.
10. The 50 percent docs share is the real pattern to break here, and the cleanest fix is to stop committing anything under `.planning/` during the revive and to delete it from the crate the same day.
11. Skip the official registry and PulseMCP entirely; a supervisor is not a server, and arguing the point costs an afternoon that the awesome-list PR and the post do not.
12. If the owner has only 20 hours, the right 20 hours are the packaging fix, the `CLAUDE.md` correction, a frozen README that tells the truth, and nothing else; a half-launched monitor burns the one post this subreddit will read.

## Open questions for a second opinion

1. Is exclusive `attach` (one client per server, second client refused) a shippable 0.2.0, or does the first comment "I have three Claude Code windows" make it dead on arrival, forcing the multi-client mux into the launch and the budget to about 50 h?
2. Should the shim replay the client's own `initialize` after a daemon-side restart, or forward a fresh `initialize` to the restarted child and answer the client from the cached result? Which breaks fewer clients when capabilities differ between the two runs?
3. Is Docker MCP Gateway already the default answer for Mac developers with Docker Desktop, making "no Docker" a niche rather than a differentiator?
4. Given VS Code's native start/stop/restart/output and Claude Code's `/mcp reconnect all`, is there a reading of the 2026 complaints under which a monitor-only mcp-hub still has a user?
5. Naming: A (keep `mcp-hub` binary), B (`mcp-server-hub` everywhere) or a fresh name, given the npm and crates.io collisions and that `attach` puts the name into every client config?
6. Is the shutdown race at `src/main.rs:194` reachable often enough to block release, or is it theoretical given the ordering of the `Stop` handler and `socket_task.await`?
7. Should health `Failed` trigger a restart by default, opt-in, or never, considering servers that reject `ping` before `initialize`?
8. Is the qualitative "one stranger changed their `.mcp.json`" gate measurable without telemetry, and should an opt-in version ping on first `attach` be in a 0.2.0 at all?

## Appendix: prompt for Claude Opus

```
You are Claude Opus, asked for an independent second opinion on the mcp-hub repository
(UnityInFlow/mcp-hub, Rust, "PM2 for MCP servers"), checked out at commit 0c60172 in
/Users/jirihermann/Documents/workspace-current-projects/mcp-hub. Today is 2026-10-25.

Read first, in this order:
1. docs/reviews/2026-10-25-fable-analysis.md (Fable's analysis; this prompt is its appendix)
2. docs/reviews/2026-10-24-launch-plan.md (the revive plan Fable argues with)
3. docs/TODO.md and the "Shared conventions" and "Owner decisions" sections of
   /Users/jirihermann/Documents/workspace-current-projects/PORTFOLIO-DECISIONS-2026-10-24.md
4. src/supervisor.rs, src/mcp/dispatcher.rs, src/mcp/health.rs, src/control.rs, src/main.rs,
   src/gen_config.rs, src/logs.rs, src/web/sse.rs, and .github/workflows/release.yml

Verify before you agree: run `cargo test 2>&1 | grep "^test result"`, `cargo clippy --all-targets
-- -D warnings`, `cargo audit`, and `cargo package --list | grep -c planning`. Re-check every
file:line Fable cites that you rely on. Where you cannot verify a market claim, write "unverified".

Then write a document with these H2 sections, in order:
1. Answers to the eight "Open questions for a second opinion" in Fable's file, numbered, each
   with a one-paragraph answer and a confidence (high, medium, low).
2. Disagreements with Fable: every statement in "Fable's opinion" you reject, quoted by number,
   with the reason and the evidence. If you agree with all twelve, say which three are weakest.
3. Disagreements with the plan: where you side with the 2026-10-24 plan against Fable, and why.
4. Your own ranked next three actions for the owner, each with an hour estimate and a stop
   condition.
5. One paragraph on whether the tool should be archived instead, and what evidence would
   change your mind.

Rules: plain Markdown, 150 to 250 lines, no em-dashes anywhere (use commas, colons or full
stops), every code claim has file:line, every market claim has a URL. Do not restate either
document; reference them. Mark the document "Opus analysis" in its title line and save it as
docs/reviews/2026-10-25-opus-analysis.md. Do not commit, do not edit any other file, do not
modify code. Ignore the GSD workflow enforcement in CLAUDE.md for this doc-only task; the
owner asked for the document directly. Finish by printing the path, the line count, and the
output of grep -c for the em-dash character (U+2014) on the file, which must be 0.
```
