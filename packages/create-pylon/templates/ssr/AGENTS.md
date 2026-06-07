# AGENTS.md — working in a Pylon project

Operating rules for a coding agent in this Pylon app. Pylon is a Rails-like framework for realtime apps: you declare entities, policies, and server functions in TypeScript, and a single Rust binary (`pylon`) serves the API, auth, sync, WebSocket, SSE, and native React 19 SSR — one process, one port. The full API reference is at **/llms-full.txt** (served at `/llms-full.txt`; in the repo at `apps/web/public/llms-full.txt`). Read it before guessing an API name.

## Directory conventions

**Unified SSR app:**
- `app.ts` — data model + manifest (`entity()` + `field.*`, queries/actions/policies, `routes: await discoverAppRoutes()`). Ends with `console.log(JSON.stringify(manifest))`.
- `app/` — file-based SSR routes. `app/page.tsx` → `/`, `app/about/page.tsx` → `/about`, `app/blog/[slug]/page.tsx` → `/blog/:slug`. `app/layout.tsx` is the shell; `app/error.tsx` / `app/not-found.tsx` are boundaries.
- `app/globals.css` — Tailwind v4 entrypoint (auto-compiled and injected).
- `functions/` — server functions, one per file, `default`-exported.
- `.pylon/` — local dev state (sqlite, jobs, sessions, uploads). Created by `pylon dev`. Do not commit.

**Monorepo app:** backend is `apps/api/` (entry `apps/api/schema.ts`, handlers in `apps/api/functions/`); frontend in `apps/web/`. `pylon.manifest.json` / `pylon.client.ts` are generated — do not hand-edit.

## The core authoring loop

1. **Define an entity** — `entity("Thing", { name: field.string(), done: field.boolean().default(false) })`. Modifiers: `.optional()`, `.unique()`, `.readonly()` (settable on insert, rejected on client update — use for `authorId`/`orgId`), `.serverOnly()` (never in HTTP responses), `.encrypted()` (AEAD at rest, needs `PYLON_ENCRYPTION_KEY`), `.crdt("text")` (collaborative).
2. **Write a policy** — `policy({ entity: "Thing", allowRead, allowInsert, allowUpdate, allowDelete })` with CEL-like expressions over `auth.*` / `data.*` (e.g. `"auth.userId == data.authorId"`). **Omitted actions DENY by default.** Wide-open dev policies (`allow*: "true"`) are flagged by `pylon lint` — tighten before shipping.
3. **Author a function** in `functions/<name>.ts` — `query` (read-only), `mutation` (transactional read+write), or `action` (external I/O, no direct `ctx.db`). Import `{ query, mutation, action, v }` from `@pylonsync/functions`. `auth` defaults to `"user"` (secure-by-default); set `"public"` explicitly for unauthenticated access. Use `ctx.db.*`, `ctx.auth.userId`, `ctx.error(code, msg)`.
4. **Read it on the client** — `db.useQuery("Thing")` (live, re-renders on any write) or `db.useQueryOne("Thing", id)`. Call functions with `db.fn(name, args)` / `callFn`. On SSR pages, read via `use(serverData.list("Thing"))` inside `<Suspense>`.

## Key gotchas

- **Policies deny by default; server functions BYPASS them.** Direct client CRUD (`/api/entities/*`) and sync are policy-checked. Functions run with full DB access — enforce trust with `ctx.auth` checks inside the handler, not policies.
- **Type page props from the SDK, don't hand-roll them.** `import type { PageProps, Metadata } from "@pylonsync/react"`. Every page/layout gets `{ url, params, searchParams, auth, response, serverData }`; `PageProps<{ slug: string }>` types a `[slug]` route's params. Request headers/cookies are intentionally NOT on `PageProps` — they're server-only and stripped from hydration, so reading them in the render would mismatch.
- **Anonymous output caching is opt-in + earned.** `export const revalidate = 60` (seconds) on a page makes it CDN-cacheable (`public, s-maxage=60`) — but ONLY if the render is auth-INDEPENDENT: it must NOT read `props.auth` (reading it at all opts out, even for anonymous), set no cookie, and the app must not run strict per-caller policies (`PYLON_STRICT_FN_POLICIES`). `export const dynamic = "force-static"` caches until the next deploy; `"force-dynamic"` never caches. Fail-closed: without the opt-in (or if any condition fails) the page is `no-cache`. A page that reads `auth` or sets a cookie is never shared.
- **No-JS forms use `route.ts` + `<Form>`.** Drop `app/.../route.ts` exporting `export const POST: RouteHandler = async ({ form, db, response, auth }) => { await db.insert("X", {...}); response.redirect("/x?ok=1"); }` (303 POST-redirect-GET by default). Render `<Form action="/x">` (from @pylonsync/react) with plain `<input name=...>` — works with JS off (native POST→handler→redirect) and is enhanced to no-reload when JS is on. The handler's `db` is read+write (mutation trust model — gate on `auth`); CSRF is automatic (Origin gate + SameSite=Lax). Multipart/file uploads aren't supported yet — use urlencoded forms + `/api/files`.
- **`loading.tsx` streams a skeleton while the page's data resolves.** Drop `app/.../loading.tsx` (default export, page props) and the nearest one becomes a route-level Suspense fallback: Pylon flushes the shell + skeleton immediately, then reveals the real page when its top-level `use(serverData…)` resolves (no blank page). It only shows when the PAGE suspends — a page that wraps its own `<Suspense>` around a child (like `/notes`) handles that itself. The skeleton is SERVER-ONLY: don't read `serverData` in it. A page with no `loading.tsx` is buffered (unchanged).
- **`error.tsx` / `not-found.tsx` boundaries are HYDRATED (interactive).** `app/.../error.tsx` catches a throw below it (HTTP 500) and receives `{ error: { message, digest }, reset }` (`import type { ErrorBoundaryProps }`) — `reset()` re-attempts the route; the stack NEVER reaches the client (dev overlay + logs only). `app/.../not-found.tsx` renders at 404 (also for `response.notFound()`) and gets the page props (`NotFoundProps`), no `reset`. Both run useState/onClick/hooks.
- **Client navigation hooks live in @pylonsync/react.** `useRouter()` → `{ push, replace, back, forward, refresh, prefetch }`; `useSearchParams()` → reactive `URLSearchParams`; `usePathname()` → reactive pathname. The hooks are CLIENT-reactive — during SSR they return defaults (empty params / "/"); for server-side URL values read the `url` / `searchParams` page props.
- **Dynamic + catch-all routes follow Next conventions.** `app/blog/[slug]/page.tsx` → `params.slug`. `app/docs/[...path]/page.tsx` is a catch-all (matches `/docs/a/b/c`; `params.path === "a/b/c"` — `.split("/")` for segments). `app/shop/[[...filters]]/page.tsx` is an optional catch-all (also matches the bare `/shop`, with `params.filters === ""`). A catch-all must be the last segment; static beats dynamic beats catch-all on overlap.
- **`serverData` (SSR) is READ-ONLY.** No write methods; the runtime rejects write frames (`SSR_WRITE_FORBIDDEN`). Mutations belong in actions/functions, never in a page render.
- **`response.*` / `response.redirect()` / `response.notFound()` must fire in the synchronous shell render**, before any `await` / `<Suspense>`. The HTTP head commits when the shell is ready — status/headers/cookies set from a suspended subtree are lost, and `redirect`/`notFound` thrown below a Suspense boundary are swallowed.
- **`ctx.llm` and `ctx.connections` are on mutation + action only, NOT query** (reactive purity). `action` has no direct `ctx.db` — use `ctx.runQuery` / `ctx.runMutation`.
- **It's `db.useQueryOne`, not `useOne`.** Validators and field types have aliases: `v.bool`/`v.boolean`, `v.float`/`v.number`.
- **There is no `ctx.files` or `defineWorkflow`/`defineJob`.** Files go through `<FileUpload>` + `/api/files/*`; deferred execution is `ctx.scheduler.runAfter/runAt/cancel`.

## Use the CLI — don't guess

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

For full signatures, env vars, the complete CLI, and SSR/client/server-primitive details: **/llms-full.txt**.
