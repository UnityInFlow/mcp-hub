# Contributing to mcp-hub

Thanks for your interest in improving mcp-hub — a local MCP server process manager (PM2 for MCP servers).

## Development

```bash
cargo build                   # compile
cargo test                    # run tests (unit + integration)
cargo clippy -- -D warnings   # lint — must pass with zero warnings
cargo fmt                     # format before every commit
cargo build --release         # optimized binary
```

Run the binary locally:

```bash
./target/debug/mcp-hub start -c mcp-hub.toml
./target/debug/mcp-hub status
```

## Code Guidelines

- Rust stable, edition 2021
- No `unwrap()` in production code — use `?` or handle the error
- `anyhow` for error handling in the binary, `thiserror` for library code
- Pattern match exhaustively — no catch-all `_` unless truly needed
- Async work goes through Tokio — no raw threads or blocking sleeps
- Web UI is server-rendered HTML (Axum + Askama) — no JavaScript frameworks

## Pull Request Process

1. Fork the repo and create a branch from `main` (e.g. `feat/log-rotation`, `fix/restart-backoff`)
2. Make your change, including tests for new behavior
3. Ensure `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` all pass locally
4. Submit a PR with a clear description of the problem and solution
5. CI runs on self-hosted runners (X64 + ARM64) — all checks must be green before merge

## Commit Convention

```
feat: add log rotation for ring buffer
fix: reduce restart backoff jitter
test: add edge cases for config reload
docs: update README with daemon mode examples
chore: bump dependencies
refactor: extract health state machine
```

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
