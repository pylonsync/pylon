You are working in `/Users/ericc59/Dev/pylon`.

Read first:

- `/Users/ericc59/Dev/pylon/README.md`
- `/Users/ericc59/Dev/pylon/ARCHITECTURE.md`
- `/Users/ericc59/Dev/pylon/ROADMAP.md`

Current state summary:

- Rust workspace exists under `crates/*`
- Bun-based TS workspace exists under `packages/*`
- canonical manifest is versioned with `manifest_version: 1`
- TS SDK currently supports:
  - `field`
  - `entity`
  - `defineRoute`
  - `query`
  - `action`
  - `policy`
  - `buildManifest`
- example app lives at:
  - `/Users/ericc59/Dev/pylon/examples/todo-app/app.ts`
- CLI commands currently include:
  - `codegen`
  - `schema check`
  - `schema diff`
  - `schema push`
  - `schema inspect`
  - `schema history`
  - `doctor`
  - `explain`
  - `init`
  - `dev`
  - `version`
- `schema push --sqlite <db-path>` applies a narrow supported SQLite plan
- SQLite introspection and history recording exist

Important constraints:

- Keep scope tight
- Do not redesign product architecture casually
- Prefer boring, explicit code
- Use Bun, not npm
- Do not add unnecessary dependencies
- Preserve current command behavior unless the task explicitly changes it
- There may be unrelated in-progress changes in the worktree; do not revert them

Default required verification:

- `cargo check`
- `cargo test`
- `bun run check`

When reporting back:

1. What changed
2. Design choices
3. Verification commands and results
4. Deferred work
5. Files changed

