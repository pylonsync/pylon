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

- Rust workspace: `crates/core`, `crates/schema`, `crates/cli` with serde-based JSON handling
- TS workspace: `packages/sdk` defines `field`, `entity`, `defineRoute`, `buildManifest`
- CLI commands: `init`, `dev`, `codegen`, `doctor`, `explain`, `version` (all support `--json`)
- Init flow: `agentdb init <path>` scaffolds a new app (accepts relative/absolute paths)
- Dev flow: `agentdb dev [app.ts]` watches, runs codegen + validation, writes manifest + client bindings
- Codegen flow: `agentdb codegen <entry.ts>` runs Bun to emit a canonical manifest
- Template: `templates/basic/` — one starter template embedded in the CLI binary
- Example app: `examples/todo-app/app.ts` is the reference fixture
- Manifest is generated, not hand-maintained: `agentdb codegen` is the source of truth
- Canonical manifest is versioned (`manifest_version: 1`); CLI validates version on load
- CLI uses `serde`/`serde_json` for all JSON parsing and output
- CLI is modular: `main.rs`, `commands/`, `manifest.rs`, `output.rs`, `bun.rs`

## First CLI Commands

- `agentdb codegen` — generate canonical manifest from TS app definition (via Bun)
- `agentdb codegen client` — generate typed TS client bindings from a manifest
- `agentdb schema check` — validate schema and manifest (first-class validation command)
- `agentdb schema diff` — compare two manifests with structured change output
- `agentdb schema push` — push schema via `--dry-run` or `--sqlite <path>` (local SQLite apply)
- `agentdb schema inspect` — inspect live SQLite DB schema (`--sqlite <path>`)
- `agentdb schema history` — view schema push audit trail (`--sqlite <path>`)
- `agentdb doctor` — environment and manifest health check
- `agentdb explain` — print structured summary of a manifest
- `agentdb init` — scaffold a new app from a template (accepts any path)
- `agentdb dev` — watch + codegen + validate loop (--once for single pass)

## Immediate Next Steps

- React SSR integration for richer static rendering
- real single-binary server (embedded runtime, no CLI needed)
- keep the TS workspace Bun-first
