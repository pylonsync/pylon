# AGENTS.md: working in a Pylon project

Pylon serves the API, auth, sync, WebSocket, SSE, and native React 19 SSR from one Rust process on one port. Treat this app as production infrastructure: it supports real auth, SQLite or Postgres, row-level policies, jobs, search, and one-command deploys. Declare entities, policies, and server functions in TypeScript; the binary handles the runtime. Read the full API reference at https://docs.pylonsync.com/llms-full.txt before guessing an API name.

## First: replace the placeholders

This template ships with placeholder branding, copy, and demo data — the example
brand **Acme**, `example.com` links, lorem-style text, sample records, and (in the
marketing templates) fake "Acme vs …" comparisons. Before building features, do
one pass to make it the real product:

- **Brand** — replace every "Acme", its logo, the `example.com` domain, the support
  email, and social links with the real name. Find them: `grep -riE "acme|example\.com" .`
- **Copy** — rewrite hero, feature, marketing, and comparison text for the actual
  product. Ship no lorem and no "Acme".
- **Demo data** — replace sample/seed records with the real domain's, or remove them.

The manifest name (`buildManifest({ name })`) is already the project name — the
placeholders live in the CONTENT. A scaffold that still says "Acme" isn't done; make
this pass before the user has to ask.

## Directory conventions

**Unified SSR app:**
- `app.ts`: data model + manifest (`entity()` + `field.*`, queries/actions/policies, `routes: await discoverAppRoutes()`). Ends with `console.log(JSON.stringify(manifest))` — that line is the manifest handoff (the CLI parses this file's stdout), not debug output. Never remove it.
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
- **Never wrap `serverData` calls in `Promise.all`.** Each method returns a thenable the handle CACHES by key — on the client it is already fulfilled, so `use()` returns synchronously. `Promise.all` builds a new, pending, uncached promise on every render, so `use()` suspends, re-renders, builds another, and the page never returns; React reports it as an async Client Component (minified error #482) and the error boundary shows a broken page. To read several things in parallel, START every call before the first `use()`, then `use()` each handle — the reads overlap and the replayed render finds each one cached:

  ```tsx
  const orgPromise = serverData.get<Org>("Org", auth.tenant_id);
  const projectsPromise = serverData.list<Project>("Project");
  const org = use(orgPromise);
  const projects = use(projectsPromise);
  ```

  Reading them one at a time (`use(serverData.a()); use(serverData.b());`) is correct but serial: each read waits for the one above it.

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
| Wire in Stack0 Analytics or Feedback | `pylon add analytics --site-key <KEY>` \| `pylon add feedback` |
| Deploy (Pylon Cloud by default) | `pylon deploy` |
| Look up an error code | `pylon explain <CODE>` |

`--json` works on every command for machine-readable output. Prefer one-shot/agent-safe flags (`pylon logs --limit N`, not a blocking `--follow`).

## Stack0 Analytics and Feedback

Neither is a package you install. Both are hosted apps you point at over HTTP, and neither is wired into this template — run `pylon add` when the product needs one.

**Analytics** — `pylon add analytics --site-key <KEY>`. Do NOT hand-write this: the relay is boilerplate whose every mistake is silent.

- The relay is a FUNCTION, not a route. `/api/fn/*` is Pylon's own namespace, so `app/api/fn/ingestEvent/route.ts` is never matched, and there is no `next.config.js` here to rewrite in. Copying the Next.js recipe gets you a file that is never called.
- It must be an `action` (only actions get `ctx.request`) and it must forward `ctx.request.rawBody`, not `args` — the beacon carries fields the arg list does not declare.
- Skipping the relay and pointing the tag straight at the Analytics host does not error either: its CORS allowlist refuses the browser's preflight, which looks exactly like nobody visiting the site.
- `stack0Analytics()` is a browser global and Pylon renders on the server. Call it from an event handler or effect, never during a render.

**Feedback** — one tag in `app/layout.tsx`; `pylon add feedback` prints it. Reading a public board takes no credential (`POST /api/fn/portalView { slug }`); writing on someone's behalf takes a guest session (`POST /api/auth/guest`, then a bearer token), not an API key. There is no API key and no MCP server in Feedback.

Full API for both: https://www.stack0.dev/docs/sdk/analytics and /docs/sdk/feedback

## Declare the env you cannot run without

`requiredEnv` in `buildManifest({...})` makes `pylon deploy` refuse to ship when the project is missing a secret:

```ts
import { requireEnv } from "@pylonsync/sdk";

requiredEnv: [
  requireEnv("SITE_URL", "this app's public origin; baked into every absolute link it serves"),
],
```

Declare the variables whose absence is SILENT — a public origin, a webhook secret, an API base URL. A missing database URL crashes on the first query and someone notices in a minute. A missing public origin gets baked into a script tag, served to a customer's website, and errors nowhere.

For full signatures, env vars, the complete CLI, and SSR/client/server-primitive details: **https://docs.pylonsync.com/llms-full.txt**.
