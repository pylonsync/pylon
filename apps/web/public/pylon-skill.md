---
name: pylon
description: Build realtime apps with Pylon — schema, policies, server functions, React client, and deployment. Use when the user is working in a Pylon project or asks to build with Pylon/Pylonsync.
---

# Pylon — Realtime backend framework

You are helping a developer build an application on **Pylon** (pylonsync.com), a realtime backend framework written in Rust with a TypeScript SDK. Pylon collapses database + API + realtime pub/sub into one process. This skill gives you the shape, conventions, and gotchas needed to build Pylon apps correctly.

## Authoritative references

This skill is a starting point, not the ceiling. When the user asks something this skill doesn't cover — a specific error code, an edge case, a feature not discussed below — fetch the source of truth:

- **Full docs index + concept map:** <https://pylonsync.com/llms.txt> — fetch this first for a condensed overview of every doc page with links.
- **Docs site:** <https://docs.pylonsync.com/> — human docs (Introduction, Quickstart, Installation, Entities, Policies, Functions, Live queries, and more).
- **Source of truth for APIs:** <https://github.com/pylonsync/pylon/tree/main/packages> — the actual `@pylonsync/sdk`, `@pylonsync/functions`, and `@pylonsync/react` source. When in doubt about a method name or signature, read the source, not your training data.
- **Working example apps:** <https://github.com/pylonsync/pylon/tree/main/examples> — 12 full apps covering CRM, ERP, chat, 3D, dashboards, etc. Best place to copy patterns.
- **This skill file (latest):** <https://pylonsync.com/pylon-skill.md> — re-fetch if the user reports the skill is out of date.

**Rule:** if you're about to use an API name or pattern you're not 100% sure exists, fetch the source or docs first. The single biggest failure mode for Pylon apps is hallucinating API names (`field.number`, `v.bool`, `relation(...)`) that look plausible but don't exist and will error at load time.

## When to use this skill

Use this skill whenever:
- The user's project has a `pylon.manifest.json`, `app.ts` importing from `@pylonsync/*`, or a `functions/` directory next to an `app.ts`.
- The user says "Pylon", "Pylonsync", "realtime backend", or asks to build a live-syncing feature.
- The user runs `pylon dev`, `pylon init`, or another `pylon` CLI command.

## Core mental model

A Pylon app is four things, all in one process:

1. **Entities** — typed tables declared in `app.ts` via the `@pylonsync/sdk` DSL. Pylon auto-migrates your database (SQLite by default, or Postgres via `DATABASE_URL`) to match.
2. **Policies** — row-level access rules evaluated as string expressions. Live alongside entities.
3. **Functions** — server TypeScript in `functions/*.ts`. Three flavors: `query`, `mutation`, `action`. RPC-called by the client.
4. **Live queries** — `db.useQuery(...)` in React subscribes to results. Pylon restreams diffs on every relevant mutation.

## Directory convention

```
my-app/
  app.ts                 # schema + policies + manifest — ENTRY POINT
  functions/             # server functions, one per file, default-exported
    createX.ts
    updateY.ts
  client/                # your React components that use @pylonsync/react
  web/                   # Vite (or Next.js) app mounting client/
    package.json
    vite.config.ts
    src/main.tsx
  package.json           # deps: @pylonsync/sdk, @pylonsync/functions
  pylon.manifest.json    # GENERATED — never edit by hand
  pylon.client.ts        # GENERATED — never edit by hand
```

`pylon dev app.ts` watches `app.ts` + `functions/` and regenerates the manifest + typed client on every save.

## Schema (`app.ts`)

Every Pylon app has an `app.ts` that imports from `@pylonsync/sdk`, declares entities + policies, and calls `buildManifest`.

```ts
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

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
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
```

**The last line is required** — `pylon dev` runs `bun run app.ts` and captures stdout as the manifest.

### Field types — EXACT API

```ts
field.string()        // TEXT
field.int()           // INTEGER 64-bit
field.float()         // REAL 64-bit
field.boolean()       // 0/1 stored as INTEGER
field.datetime()      // ISO-8601 string
field.richtext()      // long-form text
field.id("OtherEntity") // FK to another entity's id column
```

**Modifiers (chainable):**
- `.optional()` — nullable
- `.unique()` — implicit unique index on one column

**Common mistakes to avoid:**
- `field.number()` **does not exist** — use `field.float()` or `field.int()`.
- `field.bool()` **does not exist** — it's `field.boolean()`.
- `field.id()` without an entity argument **is invalid** — always pass the target entity name.

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

## Policies

Policies are boolean string expressions. They guard direct `/api/entities/*` access. Server functions bypass policies — trust yourself to check inside handlers.

**Bindings available in expressions:**
- `auth.userId` — `string | null`
- `auth.email` — `string | null`
- `auth.roles` — `string[]`
- `data.*` — the proposed row on insert/update
- `existing.*` — the current row on update/delete

**Actions:**
- `allowRead` — applied to query results; unmatched rows are filtered out silently.
- `allowInsert` / `allowUpdate` / `allowDelete` — reject the op with `POLICY_DENIED` if false.
- **Omitted actions default to deny.**

**Operators:**
```
==  !=  <  <=  >  >=  &&  ||  !  +  -  *  /  %
in            // membership: "admin" in auth.roles
ends_with     // string suffix: data.email ends_with "@example.com"
starts_with   // string prefix
```

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

// Member-of-org
policy({
  name: "doc_members",
  entity: "Document",
  allowRead: "auth.userId in existing.memberIds",
  allowInsert: "auth.userId in data.memberIds",
  allowUpdate: "auth.userId in existing.memberIds",
});

// Admin-only
policy({
  name: "audit_admin",
  entity: "AuditLog",
  allowRead: "'admin' in auth.roles",
});
```

## Functions (`functions/*.ts`)

Three flavors, all default-exported. The **filename** becomes the RPC name — `functions/createIssue.ts` is callable at `POST /api/fn/createIssue`.

### Validators — EXACT API

Import from `@pylonsync/functions`:

```ts
v.string()
v.int()
v.float()
v.boolean()          // NOT v.bool() — that does NOT exist
v.datetime()
v.id("Entity")
v.optional(v.string())
v.array(v.string())
v.literal("open")    // exact string/number/bool
v.object({ k: v.string() })
```

`v.number()` does exist as an alias for `v.float()`, but **prefer explicit `v.int()` or `v.float()`** in generated code.

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
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

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
  args: { email: v.string(), orgId: v.id("Org") },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await fetch("https://api.resend.com/emails", {
      method: "POST",
      headers: { Authorization: `Bearer ${process.env.RESEND_KEY}` },
      body: JSON.stringify({ to: args.email, subject: "Invite" }),
    });
    return { ok: true };
  },
});
```

Actions are **not transactional** — use mutations for atomic multi-row writes.

### `ctx` surface (inside handlers)

```ts
ctx.auth.userId       // string | null
ctx.auth.email        // string | null
ctx.auth.roles        // string[]

ctx.db.insert(entity, data)            // => id
ctx.db.get(entity, id)                 // => row | null
ctx.db.query(entity, filter?)          // => row[]
ctx.db.update(entity, id, patch)       // => void
ctx.db.delete(entity, id)              // => void

ctx.error("CODE", "message")           // throw typed error
ctx.schedule(delayMs, fnName, args)    // enqueue delayed call
```

### Typed errors

Always throw via `ctx.error(code, message)`. Canonical codes:
`UNAUTHENTICATED`, `POLICY_DENIED`, `NOT_FOUND`, `INVALID_ARGS`, `RATE_LIMITED`, `CONFLICT`, `INTERNAL`.

## React client

Wire up the client once, per app. In your Vite/Next entry:

```tsx
// In your app's root component or mount file
import { init, configureClient } from "@pylonsync/react";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "my-app" });
configureClient({ baseUrl: BASE_URL, appName: "my-app" });
```

`appName` must match `manifest.name` from `app.ts`.

### Live query

```tsx
import { db } from "@pylonsync/react";

function MessageList({ roomId }: { roomId: string }) {
  const { data: messages, loading } = db.useQuery("Message", { roomId });
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

## Running the app

```bash
# Terminal 1 — Pylon backend (schema watch + server on :4321)
pylon dev app.ts

# Terminal 2 — web UI
cd web && bun run dev
```

The first `pylon dev` invocation creates `.pylon/dev.db` (SQLite) and runs auto-migration. Set `DATABASE_URL=postgres://...` to target Postgres instead — the adapter is chosen at startup, and all schema/policy/function code is identical either way.

In production, use `pylon start app.ts` instead of `pylon dev`. Same server, no file watcher, blocks on the server thread so a fatal error exits the process and lets the supervisor (systemd / Docker / Fly init) restart cleanly.

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
pylon deploy --target fly        # Dockerfile + fly.toml
pylon deploy --target docker     # Dockerfile
pylon deploy --target compose    # docker-compose.yml + Dockerfile
pylon deploy --target workers    # Cloudflare wrangler.toml (experimental)
pylon deploy --target systemd    # VPS unit file
```

For Fly.io the common pattern is a 1GB volume mounted at `/data` with `auto_stop_machines = "stop"` — idle machines sleep and wake on request.

## Gotchas & rules

- **API drift** is the #1 bug cause. When writing schema, use `field.float()`/`field.int()`/`field.boolean()`. When writing validators, use `v.float()`/`v.int()`/`v.boolean()`. Do NOT use `field.number()`, `v.bool()`, or anything similar — they fail at load time.
- **Every function file must `export default`** the `mutation()/query()/action()` result. Named exports are ignored.
- **`functions/*.ts` file names are the RPC names.** `functions/create-issue.ts` would be called as `create-issue` — prefer camelCase to match JS identifier conventions.
- **Generated files** (`pylon.manifest.json`, `pylon.client.ts`) are rebuilt on every `pylon dev` invocation. Never edit by hand.
- **Workspace deps** in examples use `workspace:*` — if you scaffold outside the Pylon monorepo, replace with the published version.
- **Dev mode is generous by default** (CORS `*`, rate limits raised). Production requires explicit `PYLON_CORS_ORIGIN` — `*` is rejected.
- **Policies filter silently on read** but throw `POLICY_DENIED` on write. If a list query returns fewer rows than expected, check read policies.
- **Live queries need indexes** on filter columns. A `useQuery("Message", { roomId })` with no index on `roomId` will still work but scale O(N) per change.
- **Always call `ctx.error(code, msg)`** instead of throwing plain `Error` — plain errors become generic `HANDLER_ERROR` on the client with the real message stripped.

## Quick decision guide

| User wants | You write |
|---|---|
| A new table | New `entity(...)` in `app.ts` + matching `policy(...)` + `buildManifest({ entities: [...], policies: [...] })` |
| A list in the UI | `db.useQuery("Entity", { filter })` — make sure `filter` keys are indexed |
| A form submission / write | A `mutation()` in `functions/X.ts` + `await callFn("X", args)` in the component |
| Auth-gated writes | `if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "...")` at top of mutation handler |
| Access rules | `policy({ allowRead: "...", allowInsert: "..." })` — not middleware, not function guards |
| Email / external API | `action()` (not `mutation()`) |
| A scheduled job | `ctx.schedule(delayMs, "fnName", args)` inside a mutation |
| Deploy | `pylon deploy --target fly` then `fly deploy . --config fly.toml` |

## Before you finish a task

- Run `bun run app.ts` in the project root — if it errors, the manifest won't build and `pylon dev` will fail silently on function load.
- If you added a function, verify it's discoverable by opening the project and checking that `pylon dev` logs list your new function name in the `Loaded N functions` output.
- If you changed an entity, schema auto-migration runs — but destructive changes (dropping a required column) will refuse to apply without bumping `manifest.version`.
