---
name: pylon
description: Build real full-stack apps with Pylon — schema, policies, server functions, server-rendered React (SSR), sync, and one-command deploy. Agent-native and production-grade. Use when the user is working in a Pylon project or asks to build with Pylon/Pylonsync.
---

# Pylon — the agent-native full-stack framework

You are helping build a real application on **Pylon** (pylonsync.com), an agent-native full-stack framework written in Rust with a TypeScript SDK. **Pylon renders your React frontend AND runs your backend** — schema, live queries, auth, server functions, jobs, search, and native server-side rendering, all on one port. It's a Rust server that runs your TypeScript and SSR on Bun. SQLite by default or Postgres. It's production infrastructure, not a sandbox: real auth, row-level policies, and one-command deploy — build like it ships. This skill gives you the shape, conventions, and gotchas to build Pylon apps correctly.

**Scaffold a new app with `npm create @pylonsync/pylon@latest my-app`** — the default template is a full-stack SSR app and needs **no global install**: `@pylonsync/cli` ships as a devDependency, so `npm run dev` runs the server. Prefer this. `pylon init` (needs the global CLI) scaffolds a **backend-only** app by default — add `--frontend nextjs|react|tanstack` to pair a separate frontend host. Global CLI install: `curl -fsSL https://www.pylonsync.com/install.sh | bash`. **Bun ≥ 1.0 must be on PATH** — the Rust server spawns Bun to run your TypeScript + SSR.

## Authoritative references

This skill is a starting point, not the ceiling. When the user asks something this skill doesn't cover — a specific error code, an edge case, a feature not discussed below — fetch the source of truth:

- **Full docs index + concept map:** <https://docs.pylonsync.com/llms.txt> — fetch this first for a condensed overview of every doc page with links.
- **Docs site:** <https://docs.pylonsync.com/> — human docs covering Get started, Core concepts, Auth, Plugins, Clients, Cloud, Operations, and Compare-vs-X pages.
- **Source of truth for APIs:** <https://github.com/pylonsync/pylon/tree/main/packages> — the actual `@pylonsync/sdk`, `@pylonsync/functions`, `@pylonsync/react`, `@pylonsync/react-native`, `@pylonsync/next`, and the Swift SDK at `packages/swift/`. When in doubt about a method name or signature, read the source, not your training data.
- **Working example apps:** <https://github.com/pylonsync/pylon/tree/main/examples> — full apps covering CRM, ERP, chat, 3D, dashboards, plus `examples/swift-todo` for the iOS/macOS SDK.
- **Pylon Cloud:** <https://www.pylonsync.com> — managed Pylon. Same binary, same APIs, no infra to run.
- **This skill file (latest):** <https://www.pylonsync.com/pylon-skill.md> — re-fetch if the user reports the skill is out of date.

**Rule:** if you're about to use an API name or pattern you're not 100% sure exists, fetch the source or docs first. The SDK aliases the common naming variants (see the type table below), but anything outside that table that sounds plausible (`v.money()`, `v.enum()`, `v.timestamp()`) is probably hallucinated. (`db.useAggregate`, `db.useReactiveQuery`, `db.useSearch`, `useRoom`, `useShard` ARE real — see the realtime hooks below.)

## When to use this skill

Use this skill whenever:
- The user's project has a `pylon.manifest.json`, `app.ts` importing from `@pylonsync/*`, or a `functions/` directory next to an `app.ts`.
- The user's Swift project imports `PylonClient`, `PylonSync`, `PylonRealtime`, or `PylonSwiftUI`.
- The user says "Pylon", "Pylonsync", "realtime backend", or asks to build a live-syncing feature.
- The user runs `pylon dev`, `pylon init`, `pylon deploy`, `pylon codegen`, or another `pylon` CLI command.
- The user mentions Pylon Cloud, `www.pylonsync.com`, or `pylon deploy --target cloud`.

## Core mental model

A Pylon app is five things, all served on one port (a Rust server that runs your TypeScript and SSR on Bun):

1. **Entities** — typed tables declared in `app.ts` via the `@pylonsync/sdk` DSL. Pylon auto-migrates your database (SQLite by default, or Postgres via `DATABASE_URL`) to match.
2. **Policies** — row-level access rules evaluated as string expressions. Live alongside entities.
3. **Functions** — server TypeScript in `functions/*.ts`. Three flavors: `query`, `mutation`, `action`. RPC-called by the client.
4. **Live queries** — `db.useQuery(...)` in React subscribes to results. Pylon restreams diffs on every relevant mutation.
5. **SSR frontend** — your React app under `app/`, server-rendered by the SAME binary. Next-style file routing (`app/**/page.tsx`), `<Link>`/`<Image>`, `metadata`/`generateMetadata`, `loading.tsx`/`error.tsx`/`not-found.tsx`, `sitemap.ts`/`robots.ts`. No separate Next.js host, no `/api` proxy, one shared session on every request. (Headless/native-only apps can skip the frontend; the backend works on its own.)

## Directory convention (full-stack SSR app — the default)

```
my-app/
  app.ts                 # schema + policies + manifest — ENTRY POINT
  functions/             # server functions, one per file, default-exported
    createX.ts
  app/                   # SSR frontend — Next-style file routing, server-rendered
    layout.tsx           #   root layout (wraps every page)
    page.tsx             #   route "/"
    globals.css          #   Tailwind v4 entry; Pylon compiles + injects it
    blog/
      page.tsx           #   route "/blog"
      [slug]/page.tsx    #   dynamic route "/blog/:slug"
    sitemap.ts           #   served at /sitemap.xml (optional)
    robots.ts            #   served at /robots.txt (optional)
  components/            # your components (see the layering table below)
    ui/                  #   shadcn primitives — button, card, input, label,
                         #   select, badge, table, textarea
  components.json        # shadcn config — `bunx shadcn@latest add <name>` works
  lib/utils.ts           # `cn` (clsx + tailwind-merge)
  public/                # static assets served verbatim at the root
  package.json           # deps: @pylonsync/sdk, @pylonsync/functions, @pylonsync/react, @pylonsync/client, react, react-dom
                         #   + @pylonsync/cli as a devDependency (so `npm run dev` needs no global install)
  pylon.manifest.json    # GENERATED — never edit by hand
```

`pylon dev` watches `app.ts` + `functions/` + `app/` and regenerates the manifest, recompiles Tailwind, and live-reloads the browser on every save. The manifest's routes are discovered from `app/**` by `discoverAppRoutes()` in `app.ts` (see the scaffold). One server serves the SSR HTML, the hydration JS, the API, and the WebSocket — same origin.

(A backend-only app omits `app/` and just ships `app.ts` + `functions/`. Older apps may pair a separate Vite/Next frontend with `@pylonsync/react` + `init()`/`configureClient()` against the Pylon URL — still supported, but native SSR is the default and avoids the second host + the `/api` proxy.)

## Schema (`app.ts`)

Every Pylon app has an `app.ts` that imports from `@pylonsync/sdk`, declares entities + policies, and calls `buildManifest`.

```ts
import { entity, field, policy, buildManifest, discoverAppRoutes } from "@pylonsync/sdk";

const User = entity(
  "User",
  {
    email: field.string().unique(),
    name: field.string(),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_email", fields: ["email"], unique: true }],
  },
);

const Message = entity(
  "Message",
  {
    roomId: field.id("Room"),
    authorId: field.id("User"),
    body: field.richtext(),
    sentAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_room_time", fields: ["roomId", "sentAt"], unique: false },
    ],
  },
);

const messagePolicy = policy({
  name: "message_public_read",
  entity: "Message",
  allowRead: "true",
  allowInsert: "auth.userId == data.authorId",
  allowUpdate: "auth.userId == existing.authorId",
  allowDelete: "auth.userId == existing.authorId",
});

const manifest = buildManifest({
  name: "my-app",
  version: "0.1.0",
  entities: [User, Message],
  policies: [messagePolicy],
  queries: [],
  actions: [],
  routes: await discoverAppRoutes(),   // full-stack SSR: enumerates app/**/page.tsx (a backend-only app can pass routes: [])
});

console.log(JSON.stringify(manifest, null, 2));
```

**The last line is required** — `pylon dev` runs `bun run app.ts` and captures stdout as the manifest. Top-level `await` works because Bun runs `app.ts` as an ES module. (Note: `queries`/`actions` here are for `defineRoute`-style HTTP route bindings — your RPC functions live in `functions/*.ts` and are discovered separately, NOT listed here.)

### Field types — EXACT API

```ts
field.string()        // TEXT
field.int()           // INTEGER 64-bit
field.float()         // REAL 64-bit
field.bool()          // 0/1 stored as INTEGER; field.boolean() alias also works
field.datetime()      // ISO-8601 string
field.richtext()      // long-form text
field.id("OtherEntity") // FK to another entity's id column
```

**Modifiers (chainable):**
- `.optional()` — nullable
- `.unique()` — implicit unique index on one column
- `.default(value)` — static insert-time default (e.g. `field.bool().default(false)`)
- `.defaultNow()` — datetime defaults to insert time (e.g. `createdAt: field.datetime().defaultNow()`)
- `.owner()` — stamps the field with `auth.userId` on insert and **rejects a forged value** (403 `OWNER_MISMATCH`); also locked on update. Use for `authorId`/`buyerId`/`createdBy` so optimistic `db.insert` stays secure. Guests count (their stable guest id is stamped).
- `.serverOnly()` — never serialized in HTTP responses (secrets, `passwordHash`, `stripeCustomerId`). Still readable inside functions via `ctx.db.*`.
- `.readonly()` — settable on insert, rejected on client update (closes IDOR-via-PATCH).
- `.encrypted()` — AEAD-encrypted at rest (needs `PYLON_ENCRYPTION_KEY`).
- `.crdt("text")` — upgrade string/richtext to LoroText for collaborative merge

`field.enum(["pending", "paid", "failed"])` also exists (stored as a string with allowed-values metadata so codegen emits a precise literal union). Note: there is **no `v.enum()`** validator counterpart — validate an enum arg with `v.union(v.literal("pending"), v.literal("paid"), ...)` or a plain `v.string()`.

**Common mistakes to avoid:**
- Both `field.float()` / `field.number()` work (same type). Both `field.bool()` / `field.boolean()` work. Pick whichever reads better.
- `field.id()` without an entity argument **is invalid** — always pass the target entity name.
- Scalar fields are LWW by default. Use `field.richtext()` or `.crdt("text")` when concurrent text edits should merge.

### Indexes

Declare composite indexes in the options block. Live queries use indexed columns for fast fan-out — **index the filter columns you'll query on.**

```ts
{
  indexes: [
    { name: "by_user", fields: ["userId"], unique: false },
    { name: "by_user_created", fields: ["userId", "createdAt"], unique: false },
  ],
}
```

### Replication — `sync: false` for append-only tables

Entities default to **`sync: true`**: bulk-snapshotted and delta-streamed into
every signed-in browser's local replica. That default is right for the working
set a page renders from, and **wrong for anything that grows forever.**

```ts
const UsageWindow = entity(
  "UsageWindow",
  { projectId: field.id("Project"), periodStart: field.datetime(), requests: field.int() },
  {
    indexes: [{ name: "by_project_period", fields: ["projectId", "periodStart"], unique: true }],
    sync: false,   // read it through a function, not the replica
  },
);
```

**Rule: if the table is append-only, or no component reads it via
`db.useQuery`, set `sync: false`.** Typical: usage/metering rollups, audit and
event logs, billing history, job runs, notifier cursors, anything server-only.

`sync: false` keeps the entity out of the replica **only**. Policies, direct
reads (`/api/entities/X`), `db.useSearch`, and `ctx.db.*` inside functions are
all unchanged — so serve it with a query/action that returns the window the UI
actually renders (`limit: 168`, not the whole table).

Why it matters: the replica bootstrap walks a synced table **one cursor page at
a time, sequentially** — page N+1 needs page N's last id, so cold-start latency
is `rows / pageSize × round-trip`. A real app shipped a per-project-per-hour
rollup with the default on; a page load became ~100 chained requests at ~95ms,
about ten seconds of loading, for data no component read. It is invisible on
day one and unbearable by month three, so decide when you declare the entity.

## Policies

Policies are boolean string expressions. They guard direct `/api/entities/*` access (and sync). Server functions bypass policies — trust yourself to check inside handlers.

**Default-deny is the headline rule: an entity with NO registered policy refuses EVERY request** (`_default_deny`). This is the deliberate fix for the "forgot to lock the table" footgun — a new entity is invisible until you write its policy. For an intentionally-public entity, be explicit: `policy({ entity: "X", allowRead: "true" })`.

**Bindings available in expressions:**
- `auth.userId` — `string | null`
- `auth.isAdmin` — `boolean` (true for the `admin` role / admin token / Studio cookie)
- `auth.tenantId` — `string | null` (the selected org, for multi-tenant apps)
- `data.*` — the row: incoming payload on insert; the **current stored row** on read/update/delete
- `existing.*` — synonym for the current row (same as `data.*` on read/update/delete); use whichever reads clearer
- `now` — current UTC time as an ISO-8601 string, for time windows

Roles are checked with the **`auth.hasRole("x")` / `auth.hasAnyRole("a", "b")` functions** — there is **no `auth.roles` array and no `auth.email`** binding in policy expressions. (The SSR page `auth` prop and the session DO expose roles/email; the policy evaluator does not.)

**Actions:**
- `allowRead` — applied to query results; unmatched rows are filtered out silently.
- `allowInsert` / `allowUpdate` / `allowDelete` — reject the op with `POLICY_DENIED` if false.
- **Omitted actions default to deny.**

**Operators — the COMPLETE set (the policy language is deliberately tiny):**
```
==  !=                              // equality
<   <=   >   >=                     // ordering (numbers; timestamps if both are ISO-8601; else lexicographic)
&&  ||  !                           // boolean logic
true  false  null                  // literals
42   -3   4.5                       // numeric literals (int / float / negative)
"string"  'string'                 // string literals (either quote)
auth.hasRole("admin")              // role check
auth.hasAnyRole("admin", "owner")  // any-of role check
exists(Entity where field == <expr> [and field == <expr>]*)   // correlated subquery
```
Ordering is **deny-safe**: comparing null, booleans, or a number against a non-numeric string is always false (so an unresolvable `data.publishAt <= now` denies). Still **no `in`, no `ends_with`/`starts_with`, no arithmetic (`+ - * /`).** Membership ("is the user in this org?") is `exists(...)`, not `in`. String prefix/suffix matching still belongs in a function.

**Typical patterns:**

```ts
// Public read, author-only write
policy({
  name: "post_public",
  entity: "Post",
  allowRead: "true",
  allowInsert: "auth.userId == data.authorId",
  allowUpdate: "auth.userId == existing.authorId",
  allowDelete: "auth.userId == existing.authorId",
});

// Member-of-org — membership lives in a join entity, checked with exists()
// NOTE: `Document` must actually HAVE an `orgId` field — see below.
policy({
  name: "doc_members",
  entity: "Document",
  allowRead: "exists(OrgMember where orgId == existing.orgId and userId == auth.userId)",
  allowInsert: "exists(OrgMember where orgId == data.orgId and userId == auth.userId)",
  allowUpdate: "exists(OrgMember where orgId == existing.orgId and userId == auth.userId)",
});

// Admin-only
policy({
  name: "audit_admin",
  entity: "AuditLog",
  allowRead: "auth.isAdmin",          // or: auth.hasRole("admin")
});

// Multi-tenant row scoping
policy({
  name: "ticket_tenant",
  entity: "Ticket",
  allowRead: "auth.tenantId == existing.orgId",
  allowInsert: "auth.tenantId == data.orgId",
});

// Time window: published posts are public; scheduled ones stay hidden until their time
policy({
  name: "post_published",
  entity: "Post",
  allowRead: "data.publishAt <= now || auth.userId == data.authorId",
});

// Threshold: only show rows at or above a priority
policy({
  name: "ticket_priority",
  entity: "Ticket",
  allowRead: "data.priority >= 3",
});
```

**Gotcha: a policy that names a field the entity doesn't have denies
everything, silently.** An unresolvable reference is null, and null fails
closed — `exists(OrgMember where orgId == data.orgId ...)` on an entity keyed
`projectId` (no `orgId`) matches nothing, for everyone, forever. There is no
error and no warning: it reads exactly like a working membership check, and the
symptom is "the table looks empty" rather than "access denied".

Failing closed is the right default — the alternative leaks reads on a typo —
but it means **you must check the field exists on THAT entity** when you copy a
policy between entities. This shipped in a real app and stayed unnoticed
because the one page that displayed the data read it through a function
(functions bypass policies), so nothing appeared broken.

## Functions (`functions/*.ts`)

Three flavors, all default-exported. The **filename** becomes the RPC name — `functions/createIssue.ts` is callable at `POST /api/fn/createIssue`.

### Auth levels — read this before you write a public/pre-login feature

Every `query` / `mutation` / `action` carries an `auth` level. **The default is `"user"`.** The router enforces it *before* your handler runs — a caller who doesn't meet the bar gets a typed rejection and the handler never executes. The four levels:

| `auth:` | Who can call it | Use for |
|---|---|---|
| `"user"` **(default)** | A real signed-in user. **Guest sessions are REJECTED.** | Everything behind login. `ctx.auth.userId` narrows to `string`. |
| `"guest"` | Guest sessions (`/api/auth/guest`) **and** signed-in users | Pre-login state: carts, public demos, "try it before signing up". |
| `"public"` | Anyone, including fully unauthenticated | Webhook receivers, healthchecks, landing-page form submits. |
| `"admin"` | `ctx.auth.isAdmin === true` | Ops endpoints. |

**THE gotcha (this bites everyone): a guest session cannot call a default (`auth: "user"`) function.** If you mint a guest with `POST /api/auth/guest` and then `callFn("addToCart", …)`, the call fails with `401 AUTH_REQUIRED` ("requires a session — sign in or POST /api/auth/guest first") **unless that function declares `auth: "guest"`**. Public demos, pre-login carts, and anonymous "kick the tires" flows must set `auth: "guest"` (or `"public"`) on **every** function they call. Forgetting this is the #1 reason a scaffolded public demo returns 401s.

```ts
export default mutation({
  auth: "guest",                 // guest sessions AND users may call this
  args: { sku: v.string() },
  async handler(ctx, args) { /* ctx.auth.userId may be null — check it */ },
});
```

### Validators — EXACT API

Import from `@pylonsync/functions`:

```ts
v.string()
v.int()
v.number() / v.float()     // 64-bit float (both names work)
v.boolean() / v.bool()     // boolean (both names work)
v.datetime()               // ISO-8601 string
v.richtext()               // richtext string
v.id("Entity")
v.optional(v.string())
v.array(v.string())
v.literal("open")    // exact string/number/bool
v.object({ k: v.string() })
v.union(v.literal("a"), v.literal("b"))   // discriminated union / enum-of-literals
v.null()
v.any()
```

`v.float()` and `v.number()` are aliases for the same 64-bit float validator. Use whichever matches your `field.*` choice.

### Mutation pattern

```ts
// functions/createIssue.ts
import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    teamId: v.id("Team"),
    title: v.string(),
    description: v.optional(v.string()),
    priority: v.optional(v.int()),
  },
  // `auth: "user"` is the default — the framework rejects BOTH anon
  // AND guest callers BEFORE the handler runs, so `ctx.auth.userId` is
  // `string` (not nullable) here. Use `auth: "guest"` for pre-login
  // callers, `auth: "public"` for webhooks/healthchecks.
  async handler(ctx, args) {
    const id = await ctx.db.insert("Issue", {
      teamId: args.teamId,
      title: args.title,
      description: args.description ?? null,
      priority: args.priority ?? 0,
      authorId: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });

    return { id };
  },
});
```

### Query pattern

```ts
// functions/listIssues.ts
import { query, v } from "@pylonsync/functions";

export default query({
  args: { teamId: v.id("Team") },
  async handler(ctx, args) {
    return ctx.db.query("Issue", { teamId: args.teamId });
  },
});
```

Queries are **live** when called from the React hook — the client subscribes and re-runs on relevant mutations.

### Action pattern (side effects — emails, external HTTP)

```ts
// functions/sendInvite.ts
import { action, v } from "@pylonsync/functions";

export default action({
  // Default `auth: "user"` — anon POSTs to this endpoint never reach
  // the handler. CRITICAL for actions: policies don't gate them, so
  // a forgotten auth check on an action that calls Stripe/Resend/etc.
  // = open vulnerability. `auth: "public"` for webhook receivers only.
  args: { email: v.string(), orgId: v.id("Org") },
  async handler(ctx, args) {
    // Built-in transactional email — no HTTP call, uses the runtime's
    // PYLON_EMAIL_* provider (Resend / SendGrid / Stack0 / SMTP):
    await ctx.email.send(args.email, "Invite", "You're invited!");
    // (Or hit an external HTTP API directly with fetch(...) + ctx.env.KEY.)
    return { ok: true };
  },
});
```

**Actions have NO `ctx.db`.** They're for external I/O and are non-transactional. To read/write data from an action, call a registered function: `ctx.runQuery("listX", args)` / `ctx.runMutation("createX", args)` (each runs in its own transaction). Use mutations for atomic multi-row writes.

### `ctx` surface (inside handlers)

**The three ctx shapes are NOT the same.** In particular an `action` has **no `ctx.db`**, and a `query` ctx has no `error`/`scheduler`/writes. Don't reach for a field the flavor doesn't have.

```ts
// ---- On EVERY ctx (query / mutation / action) ----
ctx.auth.userId       // string | null  (narrows to string when auth: "user"/"admin")
ctx.auth.isAdmin      // boolean
ctx.auth.tenantId     // string | null  (selected org)
ctx.auth.elevate({ admin: true, reason: "..." })  // promote AFTER you verify a webhook/HMAC
ctx.env               // Record<string,string> — secrets / env (same values as process.env)
ctx.requireMember(orgId, { role })     // assert org membership/role — throws (UNAUTHENTICATED / MISSING_ORG / FORBIDDEN), fails closed
// NOTE: there is NO ctx.auth.email and NO ctx.auth.roles on the handler ctx.
// Need the email? -> const u = await ctx.db.get("User", ctx.auth.userId)  (query/mutation only)

// ---- query ctx — ctx.db is a READ-ONLY reader (no error, no scheduler, no writes) ----
ctx.db.get(entity, id)                 // => row | null
ctx.db.list(entity)                    // => row[]
ctx.db.query(entity, filter)           // => row[]   filter: { field: value, $gt, $lt, $in, $like, $order, $limit }
ctx.db.lookup(entity, field, value)    // => row | null
ctx.db.search(entity, spec)            // => { hits, facetCounts, total }  (needs the `search` plugin)
ctx.db.paginate(entity, { cursor, numItems })

// ---- mutation ctx — ctx.db ALSO has writes (transactional); + error, scheduler, llm, connections ----
ctx.db.insert(entity, data)            // => id (string)
ctx.db.update(entity, id, patch)       // => boolean (true if the row existed)
ctx.db.delete(entity, id)              // => boolean
ctx.db.link(entity, id, relation, targetId) / ctx.db.unlink(entity, id, relation)
ctx.db.advisoryLock(key)               // serialize a TOCTOU-prone quota/uniqueness check
throw ctx.error("CODE", "message")     // typed error → rolls the tx back
ctx.scheduler.runAfter(delayMs, "fnName", args)   // enqueue delayed call
ctx.scheduler.runAt(unixMs, "fnName", args)       // enqueue at a wall-clock time (Unix ms, e.g. new Date(iso).getTime())

// ---- action ctx — NO ctx.db. Read/write via runQuery/runMutation; + email, error, scheduler ----
ctx.runQuery("fnName", args)           // read by invoking a registered query
ctx.runMutation("fnName", args)        // write by invoking a registered mutation (own tx)
ctx.email.send(to, subject, body)      // transactional email via PYLON_EMAIL_* provider
throw ctx.error("CODE", "message")
ctx.request?.rawBody / ctx.request?.headers   // raw HTTP request (verify Stripe/GitHub webhook sigs)
```

`ctx.llm` (provider-abstracted completions) and `ctx.connections` (per-user OAuth tokens) are on **mutation + action** ctx only — never on `query` (reactive purity: a subscribed query must be a pure function of its `ctx.db` reads).

**Functions bypass policies** — a `mutation`/`action` that reads or writes another tenant's
rows without re-checking membership is an IDOR. Use `ctx.requireMember(orgId, { role })` (it
looks up the membership row and throws `UNAUTHENTICATED`/`FORBIDDEN`) rather than trusting
`args` or a client-supplied id.

`ctx.error` exists on **mutation/action** ctx but **not on query ctx** — a `query` handler that needs to signal "forbidden" should return a discriminated result (`{ authorized: false }`) rather than throw, or a plain throw will reach the client as a stripped generic `HANDLER_ERROR`.

### Typed errors

Always throw via `ctx.error(code, message)`. Canonical codes:
`UNAUTHENTICATED`, `POLICY_DENIED`, `NOT_FOUND`, `INVALID_ARGS`, `RATE_LIMITED`, `CONFLICT`, `INTERNAL`.

## React client (live data)

In a **full-stack SSR app you do NOT call `init`/`configureClient`** — the runtime wires the client to its own origin and injects the session on every render. Just import `db` / `callFn` and use them. `db.useQuery` is a live WebSocket subscription that updates on every relevant write.

(Only a **standalone** Vite/Next frontend needs manual wiring: `init({ baseUrl, appName })` + `configureClient({ baseUrl, appName })` once at mount, where `appName` matches `manifest.name`.)

### Live query

```tsx
import { db } from "@pylonsync/react";

function MessageList({ roomId }: { roomId: string }) {
  const { data: messages, loading } = db.useQuery("Message", { where: { roomId } });
  if (loading) return null;
  return (
    <ul>
      {messages.map((m) => <li key={m.id}>{m.body}</li>)}
    </ul>
  );
}
```

Filter keys must be indexed columns for performant fan-out.

### Calling functions

```tsx
import { callFn } from "@pylonsync/react";

async function onSend(roomId: string, body: string) {
  const { id } = await callFn("sendMessage", { roomId, body });
  return id;
}
```

### Realtime hooks beyond `db.useQuery`

`db.useQuery` is the workhorse, but the React SDK ships a full realtime surface. Pick the right primitive:

| You want | Hook | Notes |
|---|---|---|
| Live list of one entity, filtered | `db.useQuery("E", { where: {...} })` | Cached in the local replica; filters offline/sort locally for free. **The cross-tab-reliable primitive.** |
| One row by id, live | `db.useQueryOne("E", id)` | |
| Live pagination | `db.useInfiniteQuery("E", { pageSize })` | each page is its own subscription |
| Server-side join / computed / aggregate value, live | `db.useReactiveQuery("fnName", args)` | Convex-style: a `query()` handler that auto-re-runs when its dep set changes. **See the footgun below.** |
| Live count / sum / avg / groupBy | `db.useAggregate(...)` | |
| Live full-text + faceted search | `db.useSearch("E", { query, filters, facets, sort, pageSize })` | re-runs on every keystroke AND every matching write; needs the `search` plugin + `search:` on the entity |
| Optimistic write with a "ghost" row | `db.useMutation("fnName")` / `db.useEntity` | inserts locally instantly; server broadcast reconciles in place |
| Presence / cursors / typing / broadcast | `useRoom(roomId, userId, { initialPresence })` | ephemeral — `peers`, `setPresence(data)`, `broadcast(topic, data)`. NOT persisted. |
| Authoritative multiplayer sim (game/MMO tick loop) | `useShard(shardId, { subscriberId, token })` | `{ snapshot, tick, send, connected }`. The server-side sim (`SimState` tick loop) is defined in Rust (`crates/realtime`); `useShard`/`connectShard` are the TS client. |
| Connection/sync state for a status indicator | `useSyncStatus()` / `useSession()` | |

**Cross-tab realtime footgun (important).** For state that must update across *all* a user's tabs (a live counter, availability, "N slots left"), use **`db.useQuery` over a PII-free projection/aggregate entity** — NOT `db.useReactiveQuery`. Reactive server queries behave as leader-tab-only in practice (a follower tab's reactive subscription may never deliver its initial result), whereas entity sync (`db.useQuery`) reaches every tab. The pattern: keep your sensitive table `allowRead: "false"` (deny-all), and have the mutation also maintain a small public-read projection entity (e.g. `WaitlistStat { count }`, `BookedSlot { serviceId, startsAt }`) with `allowRead: "true"` + client writes denied. Subscribe the UI to the projection.

```tsx
// Live counter that updates in every tab — db.useQuery over a public projection
const { data: stat } = db.useQuery("WaitlistStat");   // allowRead:"true", count maintained server-side
const count = stat[0]?.count ?? 0;

// Presence (cursors / typing) — ephemeral, not stored
const room = useRoom(`doc:${docId}`, userId, { initialPresence: { cursor: 0 } });
room.peers.map((p) => <Cursor key={p.user_id} x={p.data.cursor} />);
room.setPresence({ cursor: caretPos });
```

Use `db.useReactiveQuery` for **server-side joins / computed values rendered in one (leader) view** (a feed that joins Post→User, a dashboard rollup) — it's the right tool there. Just don't lean on it for cross-tab fan-out of a shared scalar.

### Session / auth bootstrap (guest fallback pattern)

```tsx
import { storageKey } from "@pylonsync/react";

async function ensureGuest(): Promise<string> {
  const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
  let token = localStorage.getItem(storageKey("token"));
  let userId = localStorage.getItem(storageKey("user"));
  if (!token || !userId) {
    const res = await fetch(`${BASE_URL}/api/auth/guest`, { method: "POST" });
    const body = await res.json();
    token = body.token;
    userId = body.user_id;
    localStorage.setItem(storageKey("token"), token);
    localStorage.setItem(storageKey("user"), userId);
  }
  return userId;
}
```

The response is `{ guest: true, token, user_id }`. **Minting a guest is only half of it** — every function this guest then calls must declare `auth: "guest"` (or `"public"`). A guest token against a default `auth: "user"` function still returns `401 AUTH_REQUIRED`. See "Auth levels" above.

## Server-side rendering (full-stack frontend)

Pylon natively server-renders React from the same server that runs your backend (the Rust server spawns Bun to execute the render) — Next-style, but no Next. Routes are files under `app/`.

### Routing & components

- `app/page.tsx` → `/`, `app/blog/page.tsx` → `/blog`, `app/blog/[slug]/page.tsx` → `/blog/:slug`, `app/docs/[...slug]/page.tsx` → catch-all, `[[...slug]]` → optional catch-all.
- `app/layout.tsx` wraps every page (nest `layout.tsx` per segment for sub-layouts).
- **A page is a server component by default** (runs only on the server — no JS shipped). Add `"use client"` at the top of a file to make it (and its tree) an interactive island that hydrates in the browser. Proven pattern: a thin server `page.tsx` (for metadata + auth) that renders one `"use client"` view containing the interactive UI.
- **Compose small components; don't grow one giant page.** A page that fetches, formats, and renders in one file can't be tested without standing up the whole app, and an agent extending it has one huge file to edit. Split along the data boundary:

  | Layer | Contains | How it's tested |
  |---|---|---|
  | `lib/*.ts` | pure functions — formatting, grouping, derivation. No React, no `db`. | direct unit tests, no DOM, no mocks |
  | `components/*.tsx` | presentational. Data arrives as **props**; writes go out through **callbacks**. Never calls `db.useQuery` itself. | `render(<C {...fixtures} />)` — no mocking, because there's no data layer to mock |
  | `app/**/page.tsx` + one `"use client"` container | the **only** place that touches `db` / `callFn`. Fetch, then hand plain data down. | `pylon verify`, or a container test that mocks just this boundary |

  Keeping the data boundary in exactly one module is what makes everything below it trivially testable. The `create-pylon` starters are generated in this shape — read one for a worked example.

- **Build on the UI kit that's already there.** Scaffolded apps ship `components/ui/` with shadcn primitives (`button`, `card`, `input`, `label`, `select`, `badge`, `table`, `textarea`), `lib/utils.ts` (`cn`), and a configured `components.json`. Use them, and style with the semantic tokens (`bg-background`, `bg-card`, `text-muted-foreground`, `border`) rather than hardcoded palette classes like `bg-zinc-50` — a raw `<input>` or a fixed color renders as an unstyled control next to the kit and breaks dark mode.

  Need a primitive that isn't there (dialog, dropdown, tabs, popover, …)? Add it rather than hand-rolling:

  ```sh
  bunx shadcn@latest add dialog     # npx if you're on Node
  ```

  `components.json` is preconfigured (new-york, zinc, `@/components/ui`), so the component lands with the right imports and tokens.

- The page receives props: `{ url, params, searchParams, auth, response, serverData }` — type them with `PageProps<{ slug: string }>` from `@pylonsync/react` instead of hand-rolling. `auth` is `{ user_id, is_admin, tenant_id, roles }` resolved from the shared session. **The request's `headers` and `cookies` are intentionally NOT props** — they're server-only and stripped from the hydration payload (a session cookie must never reach client JS), so reading them in a component body would hydrate-mismatch. Read request-derived data through `serverData` or a server function.

```tsx
// app/blog/[slug]/page.tsx  (server component)
import type { Metadata, PageAuth } from "@pylonsync/react";

export function generateMetadata({ params }: { params: { slug: string } }): Metadata {
  return { title: `Post: ${params.slug}`, description: "…" };
}

export default function PostPage({ params, auth }: { params: { slug: string }; auth?: PageAuth }) {
  return <Article slug={params.slug} signedIn={Boolean(auth?.user_id)} />;
}
```

### Loading data during the server render (`serverData`)

A server component reads the DB during render through the `serverData` prop — a **read-only, policy-gated** handle (same store + policy gate as a query's `ctx.db`). Await it with React 19 `use()` inside `<Suspense>`; resolved values are replayed into the hydration payload so the client doesn't re-fetch. Writes are rejected (`SSR_WRITE_FORBIDDEN`) — mutations belong in functions.

```tsx
import { use, Suspense } from "react";
import type { PageProps } from "@pylonsync/react";

export default function Page({ serverData }: PageProps) {
  return (
    <Suspense fallback={<Skeleton />}>
      <PostList promise={serverData.list<Post>("Post")} />
    </Suspense>
  );
}
function PostList({ promise }: { promise: Promise<Post[]> }) {
  const posts = use(promise);
  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}
```

`serverData` has `get` / `list` / `lookup` / `query` / `queryGraph` / `paginate` / `search` (same read shapes as `ctx.db`). For live, client-updating data prefer a `"use client"` island with `db.useQuery` instead.

### Navigation, images, metadata

- `import { Link, Image } from "@pylonsync/react"` — `<Link href>` = client-side nav; `<Image src width height>` = a built-in optimizer (emits `srcset`; pass `priority` for LCP).
- SEO: `export const metadata: Metadata = {…}` (static) or `export async function generateMetadata(props)` (dynamic — keep it cheap, e.g. params→title). Fields: `title`, `description`, `canonical`, `robots`, `openGraph`, `twitter`, `icons`. Colocate `opengraph-image.png` / `icon.png` / `favicon.ico` next to a route and they're auto-wired.

### File conventions (all optional, walked up from the page dir)

- `loading.tsx` → a Suspense fallback (set `export const streaming = true` on a page for progressive streaming).
- `error.tsx` → error boundary; `not-found.tsx` → 404 boundary (call `notFound()` to trigger).
- `sitemap.ts` → `/sitemap.xml`, `robots.ts` → `/robots.txt`. Default export, **may be async** (enumerate dynamic pages from the DB). Types `Sitemap` / `Robots` from `@pylonsync/react`:

```ts
// app/sitemap.ts
import type { Sitemap } from "@pylonsync/react";
export default async function sitemap(): Promise<Sitemap> {
  const posts = await getPosts();
  return [
    { url: "https://x.com/", changeFrequency: "weekly", priority: 1 },
    ...posts.map((p) => ({ url: `https://x.com/blog/${p.slug}`, lastModified: p.updatedAt })),
  ];
}
// app/robots.ts → export default () => ({ rules: { userAgent: "*", allow: "/" }, sitemap: "https://x.com/sitemap.xml" })
```

### Client hooks & response control

- `import { useRouter, useSearchParams, usePathname, useParams, redirect, notFound } from "@pylonsync/react"` — for `"use client"` components.
- The `response` prop shapes the HTTP reply from a server component: `response.setStatus(404)`, `response.redirect("/login")`, `response.notFound()`, `response.setHeader(...)`, `response.setCookie(...)`.

### Caching (SSR output cache)

Pages are **dynamic by default**. Two opt-ins make a render shareable (a hit skips the Bun render entirely):

- **Anonymous cache** — `export const revalidate = 60` (seconds). Stored + reused for ALL anonymous visitors. Cached ONLY if the render never read per-request identity — reading `props.auth`, setting a cookie via `response.setCookie`, a non-200, or streaming opts it out. (Request headers/cookies aren't page props, so there's nothing to accidentally read there.) `export const dynamic = "force-static"` caches until the next deploy; `"force-dynamic"` never caches. Use for fully public pages.
- **Auth-bucketed cache** — `export const cache = "auth-bucketed"` + `export const revalidate = 60`. Caches TWO identity-free shells keyed on whether the request is signed in. Read `props.session.exists` (a binary signed-in bit, **never** identity) to render a signed-in vs signed-out shell and still get a per-bucket hit. Reading real `props.auth` still opts out (output would be identity-specific).
  - **Purity contract:** an `auth-bucketed` page MUST be a pure function of its props. NEVER stash request data (auth/cookies/headers, or values derived from them) in module/global scope and render it on a later request — that leaks across users (same rule as Next: no request data in module scope).
  - CDN: bucket responses are browser-`private` by default (the origin still skips the render). A shared-CDN hit requires the operator to set `PYLON_SSR_BUCKET_CDN=1` after adding a CDN cache-key rule on session-cookie presence.

To show signed-in nav on a cacheable page, read `props.session.exists` (bucketed) — do NOT read `props.auth` server-side (that makes the render dynamic). Resolve full identity client-side after hydration.

**"Why isn't my page caching?"** → run `pylon diagnostics` or open the dev HUD; it reports the exact verdict + reason per route (see "Running the app").

### Performance

Ordered by how much they actually cost. The first two are where nearly all real
latency lives; the rest matter once those are clean.

**1. Don't serialize waterfalls.** `serverData` reads are promises — start them
all, then await together. Awaiting one before creating the next turns two 50ms
reads into 100ms:

```tsx
// Waterfall: the second read doesn't start until the first resolves.
const org = use(serverData.get("Organization", id));
const members = use(serverData.query("OrgMember", { orgId: id }));

// Parallel: both in flight, one wait.
const [org, members] = use(
  Promise.all([
    serverData.get("Organization", id),
    serverData.query("OrgMember", { orgId: id }),
  ]),
);
```

Independent sections should be separate `<Suspense>` boundaries so a slow one
streams in without blocking the rest — that's what `export const streaming =
true` is for.

**2. Ship less to the client.** Resolved `serverData` is replayed into the
hydration payload so the client doesn't re-fetch — which means over-fetching in
a server component inflates the HTML for every visitor. Select the fields the UI
renders; don't pass whole rows into a `"use client"` island to use two of them.

**3. Never keep mutable state in module scope.** The SSR runner is warm and
import-caches route modules, so a module-level variable persists **across
requests and across users**. This is the same purity contract as the
`auth-bucketed` cache, and it's a correctness bug before it's a performance one.

**4. Prefer `db.useQuery` to per-component `callFn`.** The synced replica already
dedupes and updates live; N components each calling `callFn` on mount is N
round-trips for data the client usually already has.

**5. Client-render hygiene** — the usual React rules, and the two that bite most:
never define a component inside another component (it remounts the whole subtree
every render), and derive values during render instead of syncing them in an
effect.

For the full catalogue — bundle analysis, memoization, hydration, SVG and DOM
detail — use Vercel's `react-best-practices` skill
(<https://github.com/vercel-labs/agent-skills/tree/main/skills/react-best-practices>);
70 rules ranked by impact. It's Next-flavoured, so read `next/dynamic` as a
plain `import()` and server actions as Pylon functions.

## Running the app

```bash
# One port — a Rust server serves the SSR frontend, the API, and the WebSocket (it runs your TypeScript on Bun).
pylon dev
```

That's it — no second terminal for the UI. `pylon dev` watches `app.ts` + `functions/` + `app/`, recompiles Tailwind, and live-reloads the browser. (A backend-only app runs the same `pylon dev`.) The first run creates `.pylon/dev.db` (SQLite) and auto-migrates. Set `DATABASE_URL=postgres://...` to target Postgres instead — the adapter is chosen at startup, and all schema/policy/function/SSR code is identical either way.

In production, use `pylon start app.ts` instead of `pylon dev` (run `pylon build` first — it compiles the manifest, typed client, and SSR bundle; the deploy targets below do this for you). Same server, no file watcher, blocks on the server thread so a fatal error exits the process and lets the supervisor (systemd / Docker / Fly init) restart cleanly.

### Debugging — the dev HUD + `pylon diagnostics` (read this when iterating)

`pylon dev` surfaces the framework's hidden decisions so you don't guess. Same data, three ways:

- **Dev HUD** — a floating overlay (bottom-left, in the browser) showing each page's cache verdict + WHY, render mode + timing, sync connection + offline-outbox depth, and client error count.
- **`pylon diagnostics`** (machine-readable, for agents) — prints recent SSR renders: verdict (`cacheable` / `bucketed` / `dynamic`), the reason, and render time per route. `--json` for raw; `--port` if not 4321.
- **`GET /_pylon/dev/diagnostics`** — the JSON ring the CLI + HUD read.
- **`pylon dev` stdout** — one `[pylon:dev] SSR <route> → <verdict> · <ms> — <reason>` line per render.

When a page won't cache, returns no data, or is slow, check these FIRST — the verdict reason names the exact cause (e.g. "read props.auth", "not opted in", "the render set a cookie"). Dev-only; absent in prod.

## Deployment

Production env vars to set:

```bash
PYLON_DB_PATH=/data/pylon.db
PYLON_FILES_DIR=/data/uploads
PYLON_SESSION_DB=/data/sessions.db
PYLON_CORS_ORIGIN=https://your-web-ui.vercel.app   # EXACT origin — "*" refused in prod
PYLON_DEV_MODE=false
```

Scaffolding:

```bash
pylon deploy                     # default — actual hosted deploy to Pylon Cloud
pylon deploy --target fly        # Dockerfile + fly.toml
pylon deploy --target docker     # Dockerfile
pylon deploy --target compose    # docker-compose.yml + Dockerfile
pylon deploy --target workers    # Cloudflare wrangler.toml (experimental)
pylon deploy --target systemd    # VPS unit file
pylon deploy --target manifest   # just regenerate manifest + client bindings
```

For Fly.io the common pattern is a 1GB volume mounted at `/data` with `auto_stop_machines = "stop"` — idle machines sleep and wake on request.

### CLI ops surface (Pylon Cloud)

Once logged in (`pylon login`, or via the dashboard's "Hand off to your coding agent" card → `pylon login --code XXXX-XXXX`), the CLI covers every dashboard operation. Use these instead of clicking through `www.pylonsync.com` for anything scripted.

```bash
pylon projects list                     # all projects you can see
pylon projects create my-app            # create + provision a project, set it as context
pylon projects use my-app               # set current project for this dir
pylon secrets list / set KEY=v / rm KEY / import .env
pylon logs tail                          # 2s-polling request log
pylon status                             # uptime / requests / jobs / WS clients
pylon deployments list / rollback <id>
pylon domains list / add HOST / verify HOST / rm HOST
pylon db list / backup / restore <id>
pylon data entities / list <E> / get <E> <id>
pylon members list / invite EMAIL [role]
```

Every command accepts `--json` for piping to `jq`. Project context resolves from `--project` flag → `$PYLON_PROJECT` → `.pylon/project` file → interactive picker. The `.pylon/project` file is what `pylon projects use` writes; subsequent commands in that directory tree auto-target.

**Project creation** (pylon ≥ 0.3.317): `pylon projects create <slug> [--name <name>] [--org <org-slug>] [--region iad] [--db sqlite|postgres] [--no-wait]`. Creates the project, waits for the Fly machine to provision (~30–60s; Postgres adds a managed-DB provision), pins it as the local context, and prints the live `https://<slug>.pyln.dev` URL — so login → create → deploy runs end-to-end without the dashboard. `--org` is only needed when the account belongs to multiple orgs. On older CLI versions (or if the account has no org yet), create the project in the dashboard and run `pylon projects use <slug>` instead.

### Multi-machine (horizontal scaling)

One machine is the recommended shape (a single machine serves thousands of rps with a CDN in front) — scale up first. When you do need more than one machine (pylon ≥ 0.3.315):

```bash
DATABASE_URL=postgres://…          # entities, auth, sync, jobs, workflows
PYLON_CLUSTER_BUS=redis://…        # self-hosted realtime fanout
```

Postgres provides shared entities, auth state, sync state, jobs, and workflows. Workers claim jobs and workflows with short leases. Another machine can recover work after a lease expires. Execution is at least once, so job and workflow handlers must be idempotent. During a rolling release, each machine claims only jobs that have a local handler.

Redis relays WS/SSE change events, presence, and CRDT frames for self-hosted deployments. Pylon Cloud configures its built-in relay and does not add Redis to the project. Boot-time DDL is serialized across machines. One elected scheduler adds each cron fire to the shared queue. SQLite mode is single-machine by definition.

Known per-machine semantics (documented, not bugs): rate-limit counters are per machine (N machines ≈ N× the configured limit — set PYLON_RATE_LIMIT_MAX accordingly), and the SSR output cache is per machine (put a CDN in front).

Keeping a project current: `pylon update` bumps every @pylonsync/* dependency (workspace members included, `workspace:*` pins untouched) to the latest release and reinstalls — then `pylon verify`.

## Gotchas & rules

- **Type names** — schema (`field.*`) and validator (`v.*`) both accept two naming conventions so you don't have to remember which camp a given API belongs to:

  | Type | Schema (`@pylonsync/sdk`) | Validator (`@pylonsync/functions`) |
  |---|---|---|
  | string | `field.string()` | `v.string()` |
  | integer | `field.int()` | `v.int()` |
  | float | `field.float()` or `field.number()` | `v.float()` or `v.number()` |
  | boolean | `field.bool()` or `field.boolean()` | `v.bool()` or `v.boolean()` |
  | datetime | `field.datetime()` | `v.datetime()` or `v.string()` |
  | richtext | `field.richtext()` | `v.richtext()` or `v.string()` |
  | FK id | `field.id("X")` | `v.id("X")` |

  What still **doesn't exist**: `v.money()`, `v.enum()`, `v.timestamp()`. When in doubt, source is at <https://github.com/pylonsync/pylon/tree/main/packages>.
- **Every function file must `export default`** the `mutation()/query()/action()` result. Named exports are ignored.
- **`functions/*.ts` file names are the RPC names.** `functions/create-issue.ts` would be called as `create-issue` — prefer camelCase to match JS identifier conventions.
- **Generated files** (`pylon.manifest.json`, `pylon.client.ts`) are rebuilt on every `pylon dev` invocation. Never edit by hand.
- **Workspace deps** in examples use `workspace:*` — if you scaffold outside the Pylon monorepo, replace with the published version.
- **Dev mode is generous by default** (CORS `*`, rate limits raised). Production requires explicit `PYLON_CORS_ORIGIN` — `*` is rejected.
- **Policies filter silently on read** but throw `POLICY_DENIED` on write. If a list query returns fewer rows than expected, check read policies.
- **Live queries need indexes** on filter columns. A `db.useQuery("Message", { where: { roomId } })` with no index on `roomId` will still work but scale O(N) per change.
- **Always call `ctx.error(code, msg)`** instead of throwing plain `Error` — plain errors become generic `HANDLER_ERROR` on the client with the real message stripped.

## Quick decision guide

| User wants | You write |
|---|---|
| A new table | New `entity(...)` in `app.ts` + matching `policy(...)` + `buildManifest({ entities: [...], policies: [...] })` |
| A list in the UI | `db.useQuery("Entity", { where: {...} })` — make sure the `where` keys are indexed |
| A live counter / availability across tabs | `db.useQuery` over a public-read projection entity the mutation maintains — NOT `useReactiveQuery` |
| A server-side join / computed value, live | `db.useReactiveQuery("fnName", args)` (leader-view; see footgun) |
| Live full-text search | `db.useSearch("Entity", { query, facets })` + `search` plugin |
| Presence / cursors / typing | `useRoom(roomId, userId)` — ephemeral, not persisted |
| Multiplayer game / tick sim | `useShard(shardId, { subscriberId })` |
| A form submission / write | A `mutation()` in `functions/X.ts` + `await callFn("X", args)` in the component (or `db.useMutation` for optimistic UI) |
| Auth-gated functions | `auth: "user"` is the default on every `query` / `mutation` / `action`. Anon AND guest callers get `401 AUTH_REQUIRED` before the handler runs. `auth: "guest"` for pre-login callers (carts, public demos), `auth: "public"` for webhooks/healthchecks, `auth: "admin"` for ops. CRITICAL on actions — policies don't gate them. |
| Access rules | `policy({ allowRead: "...", allowInsert: "..." })` — not middleware, not function guards |
| Email / external API | `action()` (not `mutation()`) |
| A one-shot deferred job | `ctx.scheduler.runAfter(delayMs, "fnName", args)` (or `runAt(unixMs, ...)` — Unix ms, not an ISO string) inside a mutation/action |
| A recurring job | a **cron**: `cron("0 * * * *", "fnName")` (import `cron` from `@pylonsync/sdk`) in `buildManifest({ crons: [...] })`; point it at an `internal: true` function |
| Deploy | `pylon deploy --target fly` then `fly deploy . --config fly.toml` |

### Agent tooling — verify, policy dry-runs, MCP (pylon ≥ 0.3.313)

Three commands exist specifically so you can check your own work:

```bash
# Prove the app actually serves: boots it on a free port (or targets a
# running one with --url), checks /health, GETs every static route, and
# fetches every referenced JS/CSS asset (catches "renders but ships no
# hydration/styles"). Exit 0 only when nothing failed. --json for machine output.
pylon verify
pylon verify --url https://myapp.pyln.dev

# Deploy and only report success when THIS build is live and passes the
# same checks (waits for the flip; fails on failed/canceled builds):
pylon deploy --verify

# Dry-run a policy expression with the PRODUCTION evaluator before you
# ship it — allow/deny + the exact comparison that failed. Exit 0=allow, 1=deny.
pylon policy test 'auth.userId == data.ownerId' --auth userId=u1 --row '{"ownerId":"u2"}'
```

And a running app can be attached directly to your tool loop over MCP:

```bash
claude mcp add pylon -- pylon mcp --url http://localhost:4321
```

which exposes `pylon_schema` (entities/policies/routes/functions), `pylon_list` / `pylon_get` (entity reads — row policies apply, no extra authority), `pylon_call` (functions), `pylon_policy_test`, and `pylon_verify`.

Per-route CSS strategy: `export const inlineCss = true` on a page inlines the compiled stylesheet into that route's SSR `<head>` (kills the render-blocking round trip — right for cold-traffic landing pages); `false` forces the cached `<link>` (right for logged-in pages, where the immutable sheet is fetched once and free forever). Unset defers to `PYLON_SSR_INLINE_CSS`. Sheets over 32KB always link.

Dev-mode failures are disclosed where you'll see them: unhandled function errors return `Internal handler error (dev): <message> [at <file:line>]` in the HTTP response (prod stays masked), and a failed Tailwind compile paints a red banner on every page instead of silently serving unstyled.

## Before you finish a task

- Run `bun run app.ts` in the project root — if it errors, the manifest won't build and `pylon dev` will fail silently on function load.
- Run `pylon verify` — it boots the app and fails on any route/asset that doesn't serve. This is the cheap end-to-end check; don't skip it.
- Run `pylon lint` — flags wide-open dev policies (`allow*: "true"`) and other policy smells before they ship. Tighten what it reports.
- **Test in tiers, cheapest first.** Tier 1: pure helpers in `lib/` (milliseconds, no DOM). Tier 2: components with fixture props via `@testing-library/react` (happy-dom is preloaded through `bunfig.toml`). Tier 3: functions/policies through `pylon policy test` and `pylon verify`. If a component is hard to test, it's usually because it fetches its own data — lift that to the container.
- Run `pylon test` — discovers `*.test.ts` / `*.test.tsx` under `tests/` (or `functions/`) and runs them with Bun's test runner (`import { test, expect } from "bun:test"`).
- If you added a function, verify it's discoverable by opening the project and checking that `pylon dev` logs list your new function name in the `Loaded N functions` output.
- If you changed an entity, schema auto-migration runs — but destructive changes (dropping a required column) will refuse to apply without bumping `manifest.version`.
- If you wrote or changed a policy, dry-run it: `pylon policy test '<expr>' --auth ... --row ...` for both the allow case AND the deny case.

## Beyond the React quickstart — what's available

This skill focused on the React/TS happy path. Pylon has more — fetch the docs page when these come up:

### Auth (`/auth/*` in the docs)
- **Magic codes** (`/api/auth/magic/send` + `/verify`) — recommended sign-in flow. 6-digit, 10-min expiry, throttled.
- **Email + password** (`/api/auth/password/register` + `/login`) — Argon2id-hashed.
- **OAuth** — Google + GitHub built in (`/api/auth/login/:provider` + `/callback/:provider`). CSRF-protected via state tokens.
- **Sessions** — opaque 256-bit tokens, 30-day default. `/api/auth/refresh`, `/sessions` GET/DELETE for management.
- **Trusted server-side mint** — `POST /api/auth/sessions/trusted-mint`. HMAC-signed (`X-Pylon-Trusted-Signature: hex(HMAC_SHA256(PYLON_TRUSTED_SECRET, ts + "." + body))`), ±5min freshness window. Reach for this when another trusted system (Stripe Checkout, custom IdP) has verified the email and you want to skip the magic-link roundtrip. Opt-in: 404 unless `PYLON_TRUSTED_SECRET` is set.
- **RBAC** — roles on the session; `auth.hasRole('x')` in policies. `admin` role bypasses everything.
- **Multi-tenant** — `auth.tenantId` from `/api/auth/select-org`; row-scoped policies via `data.orgId == auth.tenantId`.
- **API keys** — via the `api_keys` plugin, scoped + rotatable + Argon2-hashed.

### Plugins (`/plugins/*` in the docs)
32 built-ins, declared in `manifest.plugins`:
- **Security**: `rate_limit`, `cors`, `csrf`, `net_guard` (SSRF defense), `totp`, `jwt`, `api_keys`, `session_expiry`, `password_auth`
- **Data hygiene**: `validation`, `slugify`, `timestamps`, `computed`, `cascade`, `versioning`, `soft_delete`, `tenant_scope`, `organizations`
- **Search & AI**: `search` (FTS5 + facets), `vector_search`, `ai_proxy`, `mcp` (Model Context Protocol server)
- **Integrations**: `file_storage` (S3/R2/Stack0), `cache`, `cache_client` (Redis), `email`, `webhooks`, `stripe`, `feature_flags`, `audit_log`

### Clients (`/clients/*` in the docs)
- `@pylonsync/sdk` — schema DSL + manifest builder
- `@pylonsync/react` — hooks (covered in this skill)
- `@pylonsync/react-native` — Expo SQLite-backed offline replica
- `@pylonsync/next` — Server Actions, RSC data fetching, middleware auth
- `@pylonsync/sync` — sync engine standalone (Vue, Svelte, Solid, vanilla)
- `@pylonsync/loro` — Loro CRDT integration for collaborative editing
- **Swift SDK** at `packages/swift/` — `PylonClient`, `PylonSync`, `PylonRealtime`, `PylonSwiftUI`. iOS 16+, macOS 13+, tvOS 16+, watchOS 9+, Linux. Codegen via `pylon codegen client --target swift`.

### Pylon Cloud
Managed Pylon at `www.pylonsync.com`. Same binary, same APIs.
- `pylon login` → `pylon projects create <slug>` → `pylon deploy`
- Custom domains via `pylon domains add`
- Environment vars via `pylon env set/list/unset`
- Includes: managed Postgres, TLS, magic-link email, OAuth (your creds), file storage, Studio, logs/metrics
- Pricing: usage-based, no monthly minimums; free tier covers small projects

### Compare-vs-X pages
If the user asks "Pylon vs Convex/Supabase/Firebase/Colyseus/Playroom/Nakama", point them at `/compare/<vendor>` in the docs — each page has a structured comparison with sources.

## Swift / iOS / macOS specifics

When the user is in a Swift project (Xcode, `Package.swift`, `*.swift` files):

- **Install** via SPM: `.package(url: "https://github.com/pylonsync/pylon-swift.git", from: "0.3.0")`
- **Auth**: `try await client.startMagicCode(email:)` then `try await client.verifyMagicCode(email:code:)`
- **Sync**: `await SyncEngine(config: cfg, client: client, persistence: SQLitePersistence(...))`, then `await engine.start()`
- **Mutations**: `await engine.insert("Todo", ["title": .string("x")])` (optimistic, queued, idempotent)
- **SwiftUI**: `@StateObject var todos = PylonQuery<Todo>(engine: engine, entity: "Todo")` — `todos.rows` re-renders on change
- **CRDTs**: `PylonLoroDoc(entity:rowId:)` then `await crdtDoc.attach(to: engine)` — uses `loro-swift` internally
- **Codegen**: `pylon codegen client manifest.json --target swift --out PylonGenerated.swift` produces typed structs + `PylonClient` extensions
- **Linux**: works via `FoundationNetworking`; needs `apt-get install libsqlite3-dev`

The Swift SDK is at full TS-sync parity — same wire format, same crash-safety, same offline behavior. CRDT logic is shared with TS via the same Rust Loro core.

Reference example: `examples/swift-todo/` is a complete SwiftUI iOS/macOS app.
