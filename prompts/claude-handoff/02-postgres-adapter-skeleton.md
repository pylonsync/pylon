Use `/Users/ericc59/Dev/statecraft/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Add the first Postgres storage adapter skeleton so SQLite is no longer the only concrete backend.

Build this exact slice:

1. Add a Postgres adapter module in `crates/storage`
- suggested name:
  - `postgres.rs`
  - `PostgresAdapter`

2. Keep it narrow and honest
- planning-only is acceptable in this slice
- do not implement full apply if it is not ready
- if you add `apply_schema`, unsupported operations should fail clearly

3. Define explicit SQL type mapping for Postgres
- keep it small and documented
- mirror SQLite-supported scalar types where reasonable

4. Add tests if practical
- unit tests around SQL generation or plan shape
- do not require a live Postgres server

5. Optional CLI integration only if low-risk
- do not broaden CLI surface much
- this slice is primarily about storage contract symmetry

Constraints:

- Keep scope tight
- No live DB dependency in tests
- No migration engine claims
- No connection pooling / async complexity

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`

