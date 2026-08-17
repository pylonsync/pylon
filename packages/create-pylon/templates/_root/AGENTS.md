# AGENTS.md: working in a Pylon project

Pylon serves the API, auth, sync, WebSocket, SSE, and native React 19 SSR from one Rust process on one port. Treat this app as production infrastructure: it supports real auth, SQLite or Postgres, row-level policies, jobs, search, and one-command deploys. Declare entities, policies, and server functions in TypeScript; the binary handles the runtime. Read the full API reference at https://docs.pylonsync.com/llms-full.txt before guessing an API name.

## Directory conventions

**Unified SSR app:**
- `app.ts`: data model + manifest (`entity()` + `field.*`, queries/actions/policies, `routes: await discoverAppRoutes()`). Ends with `console.log(JSON.stringify(manifest))`.
- `app/`: file-based SSR routes. `app/page.tsx` → `/`, `app/about/page.tsx` → `/about`, `app/blog/[slug]/page.tsx` → `/blog/:slug`. A `(group)` directory is stripped from the URL — `app/(marketing)/about/page.tsx` still serves `/about` — so a group's `layout.tsx` gives one section its own chrome (nav, footer) without changing any path; routes outside the group render without it. `app/layout.tsx` is the document shell; `app/error.tsx` / `app/not-found.tsx` are boundaries.
- `app/globals.css`: Tailwind v4 entrypoint (auto-compiled and injected).
- `functions/`: server functions, one per file, `default`-exported.
- `.pylon/`: local development state (SQLite, jobs, sessions, uploads). Created by `pylon dev`. Do not commit.

**Monorepo app:** backend is `apps/api/` (entry `apps/api/schema.ts`, handlers in `apps/api/functions/`); frontend in `apps/web/`. `pylon.manifest.json` / `pylon.client.ts` are generated; do not hand-edit them.

## The core authoring loop

1. **Define an entity:** `entity("Thing", { name: field.string(), done: field.boolean().default(false) })`. Modifiers: `.optional()`, `.unique()`, `.readonly()` (settable on insert, rejected on client update; use for `authorId`/`orgId`), `.serverOnly()` (never in HTTP responses), `.encrypted()` (AEAD at rest, needs `PYLON_ENCRYPTION_KEY`), `.crdt("text")` (collaborative). `field.json()` stores an arbitrary JSON value (object/array/scalar), parsed-on-read on every surface; validator twin is `v.json()`.
2. **Write a policy:** `policy({ entity: "Thing", allowRead, allowInsert, allowUpdate, allowDelete })` with CEL-like expressions over `auth.*` / `data.*` (e.g. `"auth.userId == data.authorId"`). Omitted actions deny by default. `pylon lint` flags wide-open development policies such as `allow*: "true"`; tighten them before shipping.
3. **Author a function** in `functions/<name>.ts`: `query` (read-only), `mutation` (transactional read+write), or `action` (external I/O, no direct `ctx.db`). Import `{ query, mutation, action, v }` from `@pylonsync/functions`. `auth` defaults to `"user"` (secure-by-default); set `"public"` explicitly for unauthenticated access. Use `ctx.db.*`, `ctx.auth.userId`, `ctx.error(code, msg)`.
4. **Read it on the client:** `db.useQuery("Thing")` (live, re-renders on any write) or `db.useQueryOne("Thing", id)`. Call functions with `db.fn(name, args)` / `callFn`. On SSR pages, read via `use(serverData.list("Thing"))` inside `<Suspense>`.

## Key gotchas

- **Policies deny by default; server functions bypass them.** Direct client CRUD (`/api/entities/*`) and sync are policy-checked. Functions run with full database access, so enforce trust with `ctx.auth` checks inside the handler.
- **`serverData` (SSR) is READ-ONLY.** No write methods; the runtime rejects write frames (`SSR_WRITE_FORBIDDEN`). Mutations belong in actions/functions, never in a page render.
- **`response.*` / `response.redirect()` / `response.notFound()` must fire in the synchronous shell render**, before any `await` or `<Suspense>`. The HTTP head commits when the shell is ready. Status, headers, and cookies set from a suspended subtree are lost, and `redirect` or `notFound` thrown below a Suspense boundary are swallowed.
- **`ctx.llm`, `ctx.rooms`, and `ctx.connections` are available on mutations and actions, not queries.** An `action` has no direct `ctx.db`; use `ctx.runQuery` or `ctx.runMutation`.
- **`ctx.llm.stream(request, onEvent)` streams tokens as they generate** and still resolves with the full response, so `stop_reason === "tool_use"` drives an agent tool loop. Events are `text_delta` / `tool_use_start` / `tool_input_delta` / `done`. Same auth + model-allowlist gating as `ctx.llm.complete`. Streaming does NOT extend the call deadline (`PYLON_FN_CALL_TIMEOUT`, 30s default) — set `timeout: <secs>` on the def for a long run.
- **`ctx.stream.write(text)` streams to the caller — and every fn stream is RESUMABLE**: the server buffers frames under a stream id (`streamFn`'s `onStreamId`), so a dropped connection or closed tab catches up via `resumeStream(id)`, including the final result after the handler finished. `ctx.rooms.broadcast(room, topic, data)` fans out to every CURRENTLY-CONNECTED subscriber (second device, second tab) but does not replay missed messages — use the stream id for anything that must survive a gap. Broadcast resolves `{ delivered: false }` when the room has no members — a no-op, not an error. Clients receive with `useRoomMessages(room, cb)` from `@pylonsync/react`; `useRoom` is the send side.
- **It's `db.useQueryOne`, not `useOne`.** Validators and field types have aliases: `v.bool`/`v.boolean`, `v.float`/`v.number`.
- **There is no `ctx.files` or `defineWorkflow`/`defineJob`.** Files go through `<FileUpload>` + `/api/files/*`; deferred execution is `ctx.scheduler.runAfter/runAt/cancel`.

## Use the CLI

| Need | Command |
|---|---|
| Run the app (SSR + API, hot reload, one port `:4321`) | `pylon dev` (or `npm run dev`) |
| Regenerate manifest + typed client | `pylon codegen` (Swift client: `pylon codegen client --target swift`) |
| Validate / diff / push schema | `pylon schema check` \| `diff` \| `push` |
| Migrations | `pylon migrate create <name>` \| `plan` \| `apply` |
| Lint policies (PYL001–PYL004) | `pylon lint --strict` |
| Tests | `pylon test` |
| Adversarial security probe | `pylon test:security` |
| Inspect cloud request logs (agent-safe) | `pylon logs --json --limit 50` |
| Inspect data / entities | `pylon data entities` \| `pylon data list <Entity>` |
| Call a function | `pylon fn <name> key=value` |
| Health snapshot | `pylon status` |
| Build for prod | `pylon build` |
| Deploy (Pylon Cloud by default) | `pylon deploy` |
| Look up an error code | `pylon explain <CODE>` |

`--json` works on every command for machine-readable output. Prefer one-shot/agent-safe flags (`pylon logs --limit N`, not a blocking `--follow`).

For full signatures, env vars, the complete CLI, and SSR/client/server-primitive details: **https://docs.pylonsync.com/llms-full.txt**.
