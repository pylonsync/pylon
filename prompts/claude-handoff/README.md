# Claude Handoff Prompts

These prompts are for continuing `pylon` development without re-deriving context from chat history.

Use order:

1. `00-shared-context.md`
2. one task prompt from `01-*.md` onward

Recommended sequence:

1. `01-schema-history-detail.md`
2. `02-postgres-adapter-skeleton.md`
3. `03-ts-client-bindings-codegen.md`
4. `04-query-action-runtime-stubs.md`
5. `05-schema-push-safety.md`
6. `06-postgres-introspection-skeleton.md`

Notes:

- The repo already has substantial in-progress changes. Do not revert unrelated work.
- Package manager is `bun`, not `npm`.
- Required verification is usually:
  - `cargo check`
  - `cargo test`
  - `bun run check`

