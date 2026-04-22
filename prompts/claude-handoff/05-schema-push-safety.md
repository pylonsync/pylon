Use `/Users/ericc59/Dev/statecraft/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Add safety rails to `schema push` for destructive or unsupported plans.

Build this exact slice:

1. Improve push output around unsupported/destructive operations
- make warnings explicit in both human and JSON output
- if a plan contains remove operations, clearly mark it as destructive

2. Add a confirmation mechanism if appropriate
- only if it can stay very small and explicit
- otherwise a warning-only pass is acceptable

3. Keep SQLite apply behavior honest
- unsupported operations must still fail clearly
- do not fake partial migration support

4. Add tests
- warning / destructive detection
- JSON shape

Constraints:

- Keep scope tight
- No real rollback
- No migration planner rewrite

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`

