# Contributing to pylon

Thanks for your interest! Here's how to get set up.

## Development setup

You need:

- Rust (stable) — `rustup install stable`
- Bun — `curl -fsSL https://bun.sh/install | bash`

Clone and build:

```sh
git clone https://github.com/ericc59/agentdb.git
cd pylon
cargo build
bun install
```

Run tests:

```sh
cargo test --workspace -- --test-threads=1
```

The `--test-threads=1` is only needed for integration tests that start real
servers. Unit tests run fine in parallel.

Run the dev server against the todo example:

```sh
cargo run -p pylon-cli -- dev examples/todo-app/app.pylon
```

## Project layout

- `crates/core` — shared types, error codes, utilities
- `crates/http` — platform-agnostic HTTP types (`HttpMethod`, `DataStore` trait)
- `crates/runtime` — SQLite-backed server (dev target)
- `crates/router` — HTTP routing logic, reused across platforms
- `crates/workers` — Cloudflare Workers adapter (experimental)
- `crates/functions` — Rust side of the TypeScript function runtime
- `crates/realtime` — sharded game/collab server
- `crates/auth`, `crates/policy`, `crates/sync` — self-describing
- `crates/cli` — the `pylon` binary
- `packages/*` — TypeScript SDK, React hooks, Next.js adapters, function runtime

## Guidelines

**Before opening a PR:**

1. `cargo fmt --all -- --check` — formatting
2. `cargo clippy --workspace --all-targets -- -D warnings` — no clippy warnings
3. `cargo test --workspace -- --test-threads=1` — tests pass
4. Add tests for new functionality
5. Update relevant README/docs

**Commit messages:** imperative present tense. `Fix FTS5 trigger on empty tables`
is good; `fixed the trigger` or `fixing trigger` is not.

**Small PRs over large ones.** A focused PR with one change + tests lands
faster than a rewrite.

## Filing an issue

- **Bug reports:** include a minimal reproduction. A failing test is ideal.
- **Feature requests:** describe the use case before the proposed API.
- **Security vulnerabilities:** email security@pylon.dev, not a GitHub issue.
  See `SECURITY.md`.

## Code style

- Naming: Rust uses `snake_case` for functions/variables, `CamelCase` for types,
  `SCREAMING_SNAKE_CASE` for constants. TypeScript uses `camelCase` for
  functions/variables, `PascalCase` for types.
- Comments: explain **why**, not **what**. The code explains what.
- Doc comments (`///`) on every public API. Include an example for anything
  non-obvious.
- No panics in server code paths. Return `Result<_, _>` and let the caller
  decide. `.unwrap()` is OK in tests and CLI entry points.

## License

By contributing, you agree that your contributions will be dual-licensed under
the MIT and Apache-2.0 licenses matching the project.
