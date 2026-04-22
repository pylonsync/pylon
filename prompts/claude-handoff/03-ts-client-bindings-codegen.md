Use `/Users/ericc59/Dev/statecraft/prompts/claude-handoff/00-shared-context.md` as shared context.

Goal:
Start the TS client bindings/codegen path from the canonical manifest.

Build this exact slice:

1. Add a first codegen command for TS bindings
- suggested CLI:
  - `statecraft codegen client <manifest> --out <path>`
- or a similarly narrow subcommand if it fits the current CLI structure better

2. Generate a minimal TS client artifact from the manifest
- include at least:
  - entity name unions or typed constants
  - query name unions or typed constants
  - action name unions or typed constants
- keep output small and deterministic

3. Do not implement runtime network clients yet
- this slice is about generated types/constants only

4. Add tests
- deterministic output
- generated file content for the todo example

5. Update docs minimally
- mention the command and what it generates

Constraints:

- Keep scope tight
- No runtime fetch layer
- No speculative SDK rewrite
- Reuse the canonical manifest as input

Required verification:

- `cargo check`
- `cargo test`
- `bun run check`
- generate client bindings from `examples/todo-app/statecraft.manifest.json`

