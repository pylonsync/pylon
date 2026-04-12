# agentdb Roadmap

## V1 Milestones

1. Workspace skeleton
2. CLI shell
3. Schema DSL and validation
4. TypeScript codegen
5. Route manifest and static generation
6. Query and action runtime
7. SQLite and Postgres adapters
8. Sync push/pull protocol
9. Web and mobile SDKs
10. Auth runtime
11. Studio inspector
12. Single-binary deploy path

## Current State

- Rust workspace: `crates/core`, `crates/schema`, `crates/cli` have real types and logic
- TS workspace: `packages/sdk` defines `field`, `entity`, `defineRoute`, `buildManifest`
- CLI commands: `init`, `codegen`, `doctor`, `explain`, `version` (all support `--json`)
- Init flow: `agentdb init <name>` scaffolds a new app with template, runs codegen automatically
- Codegen flow: `agentdb codegen <entry.ts>` runs Bun to emit a canonical manifest
- Template: `templates/basic/` — one starter template embedded in the CLI binary
- Example app: `examples/todo-app/app.ts` is the reference fixture
- Manifest is generated, not hand-maintained: `agentdb codegen` is the source of truth

## First CLI Commands

- `agentdb codegen` — generate canonical manifest from TS app definition (via Bun)
- `agentdb doctor` — validate a manifest
- `agentdb explain` — print structured summary of a manifest
- `agentdb init` — scaffold a new app from a template (implemented, `basic` template)
- `agentdb dev` — local dev server (not yet implemented)

## Immediate Next Steps

- add serde_json for robust manifest parsing (replace hand-rolled parser)
- begin codegen pipeline for TS client bindings from canonical schema
- add query and action type stubs to SDK
- keep the TS workspace Bun-first
