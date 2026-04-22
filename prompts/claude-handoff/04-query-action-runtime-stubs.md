Use `/Users/ericc59/Dev/statecraft/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Add runtime-facing stubs for queries and actions without claiming execution support yet.

Build this exact slice:

1. Add small Rust-side runtime contract types
- likely in:
  - `crates/query`
  - `crates/action`
- include:
  - query/action descriptors
  - names
  - input field metadata

2. Keep them manifest-oriented
- do not implement execution
- do not add transport
- no auth runtime yet

3. Add small TS SDK alignment if needed
- only if the manifest/runtime contract needs a small adjustment

4. Add tests
- descriptor construction / parsing / manifest alignment

Constraints:

- Keep scope tight
- No execution engine
- No network layer
- No policy runtime

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`

