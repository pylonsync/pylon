Use `/Users/ericc59/Dev/statecraft/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Improve `schema history --sqlite` so history is more useful to operators and agents without redesigning the history system.

Build this exact slice:

1. Add `--limit <n>` to `statecraft schema history`
- support:
  - `statecraft schema history --sqlite <db-path> --limit 10`
  - optional `--json`
- default remains full history
- invalid values should fail clearly

2. Add a single-entry mode
- support:
  - `statecraft schema history --sqlite <db-path> --id <entry-id>`
- human mode should show the full entry
- `--json` should emit the single history entry

3. Parse `plan_json` back into `SchemaPlan` when reading history
- update storage-layer history reading if needed
- avoid leaving it as a raw opaque string in CLI output
- if preserving the raw string is useful, include both only if it stays simple

4. Keep ordering newest-first for list mode

5. Add tests
- limit behavior
- `--id` behavior
- parsed plan availability

Constraints:

- Keep scope tight
- Do not add rollback
- Do not redesign the history table unless necessary
- Do not add Postgres work

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`
- create/apply a SQLite DB
- `cargo run -p statecraft-cli -- schema history --sqlite /tmp/statecraft-test.db --limit 1`
- `cargo run -p statecraft-cli -- schema history --sqlite /tmp/statecraft-test.db --json`

