Use `/Users/ericc59/Dev/agentdb/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Add a Postgres introspection skeleton so the storage layer has a path toward parity with SQLite.

Build this exact slice:

1. Define Postgres snapshot/introspection types in `crates/storage`
- keep them aligned conceptually with SQLite snapshot types where reasonable

2. Keep the slice honest
- SQL-query generation helpers or trait signatures are enough if live DB testing is too heavy
- do not require a running Postgres instance in CI/tests

3. If you can add a planning helper from a Postgres-like snapshot, do that
- but keep it narrow

4. Add tests around helper logic if practical

Constraints:

- Keep scope tight
- No live Postgres dependency in tests
- No full apply implementation unless trivial and honest

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`

