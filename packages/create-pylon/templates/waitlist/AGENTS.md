# AGENTS.md — working in a Pylon project

Operating rules for a coding agent in this Pylon app. You — the agent — are a first-class user of Pylon: one Rust binary (`pylon`) serves the API, auth, sync, WebSocket, SSE, and native React 19 SSR from one process on one port, so you build, run, and ship a whole app without wiring services together or leaving the codebase. This is production infrastructure, not a sandbox — real auth, SQLite or Postgres, row-level policies, jobs, search, and one-command deploy — so build like it ships. You declare entities, policies, and server functions in TypeScript; the binary does the rest. The full API reference is the **llms-full.txt** at https://docs.pylonsync.com/llms-full.txt — read it before guessing an API name.

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
- **Anonymous output caching is opt-in + earned.** `export const revalidate = 60` (seconds) on a page makes it CDN-cacheable (`public, s-maxage=60`) — but ONLY if the render is auth-INDEPENDENT: it must NOT read `props.auth` (reading it at all opts out, even for anonymous), set no cookie, and the app must not run strict per-caller policies (`PYLON_STRICT_FN_POLICIES`). `export const dynamic = "force-static"` caches until the next deploy; `"force-dynamic"` never caches. Fail-closed: without the opt-in (or if any condition fails) the page is `no-cache`. A page that reads `auth` or sets a cookie is never shared. The SAME earned render is also kept in an **origin disk cache** (`.pylon/.cache/ssr`): a cookie-less GET with no query string is served straight off disk for the TTL — skipping the render entirely — then re-rendered live when stale. The disk cache is namespaced per deploy (wiped on each new build) and OFF in `pylon dev` (so an edit is never masked by a stale entry); invalidation is by the `revalidate` TTL or the next deploy.
- **No-JS forms use `route.ts` + `<Form>`.** Drop `app/.../route.ts` exporting `export const POST: RouteHandler = async ({ form, db, response, auth }) => { await db.insert("X", {...}); response.redirect("/x?ok=1"); }` (303 POST-redirect-GET by default). Render `<Form action="/x">` (from @pylonsync/react) with plain `<input name=...>` — works with JS off (native POST→handler→redirect) and is enhanced to no-reload when JS is on. The handler's `db` is read+write (mutation trust model — gate on `auth`); CSRF is automatic (Origin gate + SameSite=Lax). Multipart/file uploads aren't supported yet — use urlencoded forms + `/api/files`.
- **`loading.tsx` streams a skeleton while the page's data resolves.** Drop `app/.../loading.tsx` (default export, page props) and the nearest one becomes a route-level Suspense fallback: Pylon flushes the shell + skeleton immediately, then reveals the real page when its top-level `use(serverData…)` resolves (no blank page). It only shows when the PAGE suspends — a page that wraps its own `<Suspense>` around a child (like `/dashboard` in this template) handles that itself. The skeleton is SERVER-ONLY: don't read `serverData` in it. A page with no `loading.tsx` is buffered (unchanged).
- **`export const streaming = true` streams a page's OWN inner `<Suspense>` boundaries.** Without it (and without a `loading.tsx`), a page is BUFFERED — the whole document, including suspended children, resolves before the first byte. Opt in and the shell + each inner `<Suspense>` fallback flush immediately, then each boundary's real content streams in as its data resolves (multi-boundary progressive streaming). It's opt-in because it changes the response timing contract: a streaming render commits its HTTP head BEFORE suspended subtrees finish, so (a) it's never CDN/disk cacheable — don't combine with `export const revalidate`; (b) `response.setStatus/setCookie/redirect/notFound` only take effect from the SYNCHRONOUS shell render — a call from inside a suspended subtree is dropped (the runtime logs a loud warning naming what was lost); (c) a `throw` from a deep `<Suspense>` child resolves via its nearest `error.tsx` at HTTP 200, not a 5xx. Hydration is clean for any number of boundaries (the data blob ships before hydration runs). Type the config with `import type { RouteSegmentConfig } from "@pylonsync/react"`.
- **`error.tsx` / `not-found.tsx` boundaries are HYDRATED (interactive).** `app/.../error.tsx` catches a throw below it (HTTP 500) and receives `{ error: { message, digest }, reset }` (`import type { ErrorBoundaryProps }`) — `reset()` re-attempts the route; the stack NEVER reaches the client (dev overlay + logs only). `app/.../not-found.tsx` renders at 404 (also for `response.notFound()`) and gets the page props (`NotFoundProps`), no `reset`. Both run useState/onClick/hooks.
- **Client navigation hooks live in @pylonsync/react.** `useRouter()` → `{ push, replace, back, forward, refresh, prefetch }`; `useSearchParams()` → reactive `URLSearchParams`; `usePathname()` → reactive pathname. The hooks are CLIENT-reactive — during SSR they return defaults (empty params / "/"); for server-side URL values read the `url` / `searchParams` page props.
- **Dynamic + catch-all routes follow Next conventions.** `app/blog/[slug]/page.tsx` → `params.slug`. `app/docs/[...path]/page.tsx` is a catch-all (matches `/docs/a/b/c`; `params.path === "a/b/c"` — `.split("/")` for segments). `app/shop/[[...filters]]/page.tsx` is an optional catch-all (also matches the bare `/shop`, with `params.filters === ""`). A catch-all must be the last segment; static beats dynamic beats catch-all on overlap.
- **`serverData` (SSR) is READ-ONLY.** No write methods; the runtime rejects write frames (`SSR_WRITE_FORBIDDEN`). Mutations belong in actions/functions, never in a page render.
- **`response.*` / `response.redirect()` / `response.notFound()` must fire in the synchronous shell render**, before any `await` / `<Suspense>`. The HTTP head commits when the shell is ready — status/headers/cookies set from a suspended subtree are lost, and `redirect`/`notFound` thrown below a Suspense boundary are swallowed.
- **`ctx.llm` and `ctx.connections` are on mutation + action only, NOT query** (reactive purity). `action` has no direct `ctx.db` — use `ctx.runQuery` / `ctx.runMutation`.
- **It's `db.useQueryOne`, not `useOne`.** Validators and field types have aliases: `v.bool`/`v.boolean`, `v.float`/`v.number`.
- **There is no `ctx.files` or `defineWorkflow`/`defineJob`.** Files go through `<FileUpload>` + `/api/files/*`. Deferred (one-shot) execution is `ctx.scheduler.runAfter/runAt/cancel`. Recurring work is a **cron**: `cron("0 * * * *", "fnName")` in `buildManifest({ crons: [...] })` (import `cron` from `@pylonsync/sdk`) — it fires the named function (make it `internal: true`) on the schedule; the function runs with anonymous auth — its own `ctx.db.*` is server-side (not policy-gated), so write directly; only `ctx.auth.elevate({ admin: true, reason: "..." })` (reason mandatory) to chain an `internal: true` function via `ctx.scheduler`.

## Testing

`pylon test` discovers every `*.test.ts` / `*.test.tsx` file under `tests/` (or `functions/`) and runs it with **Bun's test runner** (`import { test, expect } from "bun:test"`). Run the suite with `pylon test` (or `npm test`); filter with `pylon test <substring>`. This template ships `bunfig.toml` + `tests/setup.ts` (registers happy-dom) so component tests render out of the box, plus starter tests under `tests/` — replace them with your own.

**Tier 1 — pure logic (reach for this first).** Keep the decisions that matter — access/plan gating, pricing, credit math, validation, formatting — in pure functions in `lib/`, and test those exhaustively. No server, instant, and it's where the real bugs live. Keep your `query`/`mutation`/`action` handlers as thin wrappers around them, so the logic is testable without a running app.

```ts
import { expect, test } from "bun:test";
import { productBySlug } from "../lib/site.config";

test("unknown slug → undefined", () => {
  expect(productBySlug("nope")).toBeUndefined();
});
```

**Tier 2 — React components.** `@testing-library/react` + happy-dom are already wired (`tests/setup.ts`). Render and assert. The template uses the classic JSX transform, so add `import React from "react"` in `.tsx` tests. For a component that reads Pylon data hooks, **mock the boundary**, then dynamic-`import` the component so the mock is in place first:

```tsx
import { test, expect, mock } from "bun:test";
import React from "react";
import { render, screen } from "@testing-library/react";

mock.module("@pylonsync/react", () => ({
  db: { useQuery: () => ({ data: [{ id: "1", name: "Acme" }], loading: false }) },
}));
const { OrgList } = await import("../app/orgs/org-list"); // your component

test("renders orgs from the query", () => {
  render(<OrgList />);
  expect(screen.getByText("Acme")).toBeDefined();
});
```

**Tier 3 — functions over HTTP (only when Tier 1 can't cover it).** A handler's full behavior (policies, `ctx.db`, auth) lives in the running app. Start `pylon dev` in another terminal and call the API; `resetDb()` from `@pylonsync/functions` clears the in-memory DB between cases (no-ops if the server's down, refuses production).

```ts
import { afterEach, expect, test } from "bun:test";
import { resetDb } from "@pylonsync/functions";

const BASE = "http://localhost:4321";
afterEach(() => resetDb(BASE)); // or installTestIsolation(BASE) once at top-of-file

test("createThing then read it back", async () => {
  const t = await fetch(`${BASE}/api/fn/createThing`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "hello" }),
  }).then((r) => r.json());
  const rows = await fetch(`${BASE}/api/entities/Thing`).then((r) => r.json());
  expect(rows.some((r: { id: string }) => r.id === t.id)).toBe(true);
});
```

`pylon test:security` is a separate adversarial probe — it hits a running app and reports auth/policy holes (run `pylon dev`, then `pylon test:security`).

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

For full signatures, env vars, the complete CLI, and SSR/client/server-primitive details: **https://docs.pylonsync.com/llms-full.txt**.
