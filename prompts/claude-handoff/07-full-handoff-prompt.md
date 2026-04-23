You are taking over implementation in `/Users/ericc59/Dev/pylon`.

Read:

- `/Users/ericc59/Dev/pylon/README.md`
- `/Users/ericc59/Dev/pylon/ARCHITECTURE.md`
- `/Users/ericc59/Dev/pylon/ROADMAP.md`
- `/Users/ericc59/Dev/pylon/prompts/claude-handoff/00-shared-context.md`

Current state:

- The repo has a real Rust/Bun workspace
- The manifest contract is versioned and validated
- The CLI is substantial and schema-oriented
- SQLite storage planning, apply, introspection, and history exist
- The next sensible work is one of:
  - improve schema history detail
  - add Postgres adapter symmetry
  - start TS client bindings codegen

Task selection:

1. First inspect current repo state and avoid reverting unrelated work.
2. Pick one prompt from:
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/01-schema-history-detail.md`
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/02-postgres-adapter-skeleton.md`
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/03-ts-client-bindings-codegen.md`
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/04-query-action-runtime-stubs.md`
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/05-schema-push-safety.md`
   - `/Users/ericc59/Dev/pylon/prompts/claude-handoff/06-postgres-introspection-skeleton.md`
3. Implement it with boring, explicit code.
4. Run:
   - `cargo check`
   - `cargo test`
   - `bun run check`
5. Report:
   - what changed
   - design choices
   - verification
   - deferred work
   - files changed

Important constraints:

- Use Bun, not npm
- Do not redesign the architecture casually
- Do not add unnecessary dependencies
- Do not claim runtime support that does not exist
