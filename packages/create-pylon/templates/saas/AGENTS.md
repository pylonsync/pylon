# AGENTS.md: working in a Pylon project

Pylon serves the API, auth, sync, WebSocket, SSE, and native React 19 SSR from one Rust process on one port. Treat this app as production infrastructure: it supports real auth, SQLite or Postgres, row-level policies, jobs, search, and one-command deploys. Declare entities, policies, and server functions in TypeScript; the binary handles the runtime. Read the full API reference at https://docs.pylonsync.com/llms-full.txt before guessing an API name.

## Directory conventions

**Unified SSR app:**
- `app.ts`: data model + manifest (`entity()` + `field.*`, queries/actions/policies, `routes: await discoverAppRoutes()`). Ends with `console.log(JSON.stringify(manifest))`.
- `app/`: file-based SSR routes. `app/page.tsx` → `/`, `app/about/page.tsx` → `/about`, `app/blog/[slug]/page.tsx` → `/blog/:slug`. `app/layout.tsx` is the shell; `app/error.tsx` / `app/not-found.tsx` are boundaries.
- `app/globals.css`: Tailwind v4 entrypoint (auto-compiled and injected).
- `functions/`: server functions, one per file, `default`-exported.
- `.pylon/`: local dev state (SQLite, jobs, sessions, uploads). Created by `pylon dev`. Do not commit.

**Monorepo app:** backend is `apps/api/` (entry `apps/api/schema.ts`, handlers in `apps/api/functions/`); frontend in `apps/web/`. `pylon.manifest.json` / `pylon.client.ts` are generated; do not hand-edit.

## The core authoring loop

1. **Define an entity:** `entity("Thing", { name: field.string(), done: field.boolean().default(false) })`. Modifiers: `.optional()`, `.unique()`, `.readonly()` (settable on insert, rejected on client update; use for `authorId`/`orgId`), `.serverOnly()` (never in HTTP responses), `.encrypted()` (AEAD at rest, needs `PYLON_ENCRYPTION_KEY`), `.crdt("text")` (collaborative).
2. **Write a policy:** `policy({ entity: "Thing", allowRead, allowInsert, allowUpdate, allowDelete })` with CEL-like expressions over `auth.*` / `data.*` (e.g. `"auth.userId == data.authorId"`). Omitted actions deny by default. `pylon lint` flags wide-open development policies such as `allow*: "true"`; tighten them before shipping.
3. **Author a function** in `functions/<name>.ts`: `query` (read-only), `mutation` (transactional read+write), or `action` (external I/O, no direct `ctx.db`). Import `{ query, mutation, action, v }` from `@pylonsync/functions`. `auth` defaults to `"user"` (secure-by-default); set `"public"` explicitly for unauthenticated access. Use `ctx.db.*`, `ctx.auth.userId`, `ctx.error(code, msg)`.
4. **Read it on the client:** `db.useQuery("Thing")` (live, re-renders on any write) or `db.useQueryOne("Thing", id)`. Call functions with `db.fn(name, args)` / `callFn`. On SSR pages, read via `use(serverData.list("Thing"))` inside `<Suspense>`.

## Key gotchas

- **Policies deny by default; server functions bypass them.** Direct client CRUD (`/api/entities/*`) and sync are policy-checked. Functions run with full database access. Enforce trust inside the handler with `ctx.auth` checks, or use `ctx.requireMember(orgId, { role: ["owner", "admin"] })` for organization membership and role gates. It throws `FORBIDDEN` for non-members and works in queries, mutations, and actions. Do not hand-roll a member lookup; this primitive fails closed.
- **Type page props from the SDK, don't hand-roll them.** `import type { PageProps, Metadata } from "@pylonsync/react"`. Every page/layout gets `{ url, params, searchParams, auth, response, serverData }`; `PageProps<{ slug: string }>` types a `[slug]` route's params. Request headers/cookies are intentionally NOT on `PageProps`; they're server-only and stripped from hydration, so reading them in the render would mismatch.
- **Anonymous output caching is opt-in and conditional.** `export const revalidate = 60` makes a page CDN-cacheable (`public, s-maxage=60`) only when the render is auth-independent: it must not read `props.auth`, set a cookie, or run with strict per-caller policies (`PYLON_STRICT_FN_POLICIES`). `export const dynamic = "force-static"` caches until the next deploy; `"force-dynamic"` never caches. When any condition fails, the page is `no-cache`. Eligible renders also use the origin disk cache at `.pylon/.cache/ssr`: a cookie-less GET without a query string is served from disk for the TTL and rerendered when stale. The cache is namespaced per deploy, cleared by each build, disabled in `pylon dev`, and invalidated by the `revalidate` TTL or the next deploy.
- **No-JS forms use `route.ts` and `<Form>`.** Add `app/.../route.ts` exporting `export const POST: RouteHandler = async ({ form, db, response, auth }) => { await db.insert("X", {...}); response.redirect("/x?ok=1"); }` (303 POST-redirect-GET by default). Render `<Form action="/x">` from `@pylonsync/react` with plain `<input name=...>`. It uses native POST → handler → redirect without JavaScript and no-reload enhancement with JavaScript. The handler's `db` is read+write under the mutation trust model, so gate it on `auth`. CSRF protection is automatic through the Origin gate and SameSite=Lax. Multipart uploads are not supported yet; use URL-encoded forms and `/api/files`.
- **`loading.tsx` streams a skeleton while the page's data resolves.** Drop `app/.../loading.tsx` (default export, page props) and the nearest one becomes a route-level Suspense fallback: Pylon flushes the shell + skeleton immediately, then reveals the real page when its top-level `use(serverData…)` resolves (no blank page). It only shows when the PAGE suspends; a page that wraps its own `<Suspense>` around a child (like `/dashboard` in this template) handles that itself. The skeleton is SERVER-ONLY: don't read `serverData` in it. A page with no `loading.tsx` is buffered (unchanged).
- **`export const streaming = true` streams a page's inner `<Suspense>` boundaries.** Without it or a `loading.tsx`, the page is buffered until all suspended children resolve. With it, the shell and fallbacks flush immediately, then each boundary streams its content. Streaming commits the HTTP head before suspended subtrees finish, so the page is never CDN- or disk-cacheable; do not combine it with `export const revalidate`. Calls to `response.setStatus`, `setCookie`, `redirect`, or `notFound` only take effect during the synchronous shell render. A call from a suspended subtree is dropped and logged. An error from a deep `<Suspense>` child resolves through the nearest `error.tsx` at HTTP 200 rather than 5xx. Type the config with `import type { RouteSegmentConfig } from "@pylonsync/react"`.
- **`error.tsx` / `not-found.tsx` boundaries are HYDRATED (interactive).** `app/.../error.tsx` catches a throw below it (HTTP 500) and receives `{ error: { message, digest }, reset }` (`import type { ErrorBoundaryProps }`); `reset()` re-attempts the route; the stack NEVER reaches the client (dev overlay + logs only). `app/.../not-found.tsx` renders at 404 (also for `response.notFound()`) and gets the page props (`NotFoundProps`), no `reset`. Both run useState/onClick/hooks.
- **Client navigation hooks live in @pylonsync/react.** `useRouter()` → `{ push, replace, back, forward, refresh, prefetch }`; `useSearchParams()` → reactive `URLSearchParams`; `usePathname()` → reactive pathname. The hooks are CLIENT-reactive; during SSR they return defaults (empty params / "/"); for server-side URL values read the `url` / `searchParams` page props.
- **Dynamic + catch-all routes follow Next conventions.** `app/blog/[slug]/page.tsx` → `params.slug`. `app/docs/[...path]/page.tsx` is a catch-all (matches `/docs/a/b/c`; `params.path === "a/b/c"`; `.split("/")` for segments). `app/shop/[[...filters]]/page.tsx` is an optional catch-all (also matches the bare `/shop`, with `params.filters === ""`). A catch-all must be the last segment; static beats dynamic beats catch-all on overlap.
- **`serverData` (SSR) is READ-ONLY.** No write methods; the runtime rejects write frames (`SSR_WRITE_FORBIDDEN`). Mutations belong in actions/functions, never in a page render.
- **`response.*` / `response.redirect()` / `response.notFound()` must fire in the synchronous shell render**, before any `await` / `<Suspense>`. The HTTP head commits when the shell is ready; status/headers/cookies set from a suspended subtree are lost, and `redirect`/`notFound` thrown below a Suspense boundary are swallowed.
- **`ctx.llm`, `ctx.rooms`, and `ctx.connections` are on mutation + action only, NOT query** (reactive purity). `action` has no direct `ctx.db`; use `ctx.runQuery` / `ctx.runMutation`.
- **`ctx.llm.stream(request, onEvent)` streams tokens as they generate** and still resolves with the full response, so `stop_reason === "tool_use"` drives an agent tool loop. Events are `text_delta` / `tool_use_start` / `tool_input_delta` / `done`. Same auth + model-allowlist gating as `ctx.llm.complete`. Streaming does NOT extend the call deadline (`PYLON_FN_CALL_TIMEOUT`, 30s default) — set `timeout: <secs>` on the def for a long run.
- **`ctx.stream.write(text)` reaches only the one client holding the HTTP response; `ctx.rooms.broadcast(room, topic, data)` reaches every subscriber of the room.** Broadcast is what makes agent output survive a closed tab or reach a second device. It resolves `{ delivered: false }` when the room has no members — a no-op, not an error. Clients receive with `useRoomMessages(room, cb)` from `@pylonsync/react`; `useRoom` is the send side.
- **It's `db.useQueryOne`, not `useOne`.** Validators and field types have aliases: `v.bool`/`v.boolean`, `v.float`/`v.number`.
- **Use the supported file and scheduling APIs.** Files go through `<FileUpload>` and `/api/files/*`; there is no `ctx.files`. One-shot work uses `ctx.scheduler.runAfter`, `runAt`, or `cancel`; there is no `defineWorkflow` or `defineJob`. Recurring work uses `cron("0 * * * *", "fnName")` in `buildManifest({ crons: [...] })`, imported from `@pylonsync/sdk`. Make the target function `internal: true`. It runs with anonymous auth, but its own `ctx.db.*` calls are server-side and bypass policies. Use `ctx.auth.elevate({ admin: true, reason: "..." })`, with a mandatory reason, only when chaining another internal function through `ctx.scheduler`.

## Testing

`pylon test` discovers every `*.test.ts` / `*.test.tsx` file under `tests/` (or `functions/`) and runs it with **Bun's test runner** (`import { test, expect } from "bun:test"`). Run the suite with `pylon test` (or `npm test`); filter with `pylon test <substring>`. This template ships `bunfig.toml` + `tests/setup.ts` (registers happy-dom) so component tests render out of the box, plus starter tests under `tests/`; replace them with your own.

**Tier 1: pure logic (start here).** Keep access and plan gating, pricing, credit math, validation, and formatting in pure functions under `lib/`, and test them exhaustively. These tests need no server and run instantly. Keep `query`, `mutation`, and `action` handlers as thin wrappers so their decision logic remains testable without a running app.

```ts
import { expect, test } from "bun:test";
import { productBySlug } from "../lib/site.config";

test("unknown slug → undefined", () => {
  expect(productBySlug("nope")).toBeUndefined();
});
```

**Tier 2: React components.** `@testing-library/react` and happy-dom are already wired through `tests/setup.ts`. Render and assert. The template uses the classic JSX transform, so add `import React from "react"` in `.tsx` tests. For a component that reads Pylon data hooks, mock the boundary, then dynamically import the component so the mock is in place first:

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

**Tier 3: functions over HTTP.** Use this tier when pure logic tests cannot cover the behavior. A handler's policies, `ctx.db` calls, and auth run in the app. Start `pylon dev` in another terminal and call the API. `resetDb()` from `@pylonsync/functions` clears the in-memory database between cases; it does nothing when the server is down and refuses to run in production.

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

`pylon test:security` is a separate adversarial probe; it hits a running app and reports auth/policy holes (run `pylon dev`, then `pylon test:security`).

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
