# Chat — pylon example

A Slack-style chat app in one manifest + six functions + one React file.

Demonstrates:

- **Live sync** — messages appear in every connected client instantly. No
  explicit push code; the change log fan-out handles it.
- **Rooms API** — `useRoom("channel:<id>")` gives you per-channel presence
  ("3 others here") and typing indicators ("Alice is typing…").
- **Tenant isolation** — `Workspace` is the tenant. Every scoped entity has
  a `tenantId` field; the `tenant_scope` plugin stamps it automatically on
  inserts and rejects cross-tenant reads/writes. Default posture — no code
  in the functions has to call `.where({ tenantId })`.
- **Transactional mutations** — `sendMessage.ts` is atomic: the channel
  lookup, membership check, and message insert all roll back together if
  any step throws.
- **Optimistic UI** — `useMutation("sendMessage")` clears the input
  instantly; the sync engine reconciles when the server confirms.
- **Reactions with racing double-tap** — `toggleReaction.ts` handles
  concurrent "+👍" clicks safely via the `(messageId, userId, emoji)`
  unique index.
- **Read markers** — `markChannelRead.ts` upserts one marker per
  (user, channel); the client compares `lastReadAt` against message
  timestamps to render unread counts.

## Run it

Two terminals:

**Terminal 1 — pylon backend (port 4321):**

```sh
cd examples/chat
pylon dev app.ts
```

**Terminal 2 — Vite dev server for the React UI (port 5173):**

```sh
cd examples/chat/web
bun install     # first time only
bun run dev
```

Open two browser windows at `http://localhost:5173`. Sign in as different
emails in each; send messages and watch them land live.

Studio at `http://localhost:4321/studio` lets you inspect rows directly
(requires admin token in non-dev mode).

## What to read first

| File | Why |
|---|---|
| `pylon.manifest.json` | Data model — 7 entities, 4 policies |
| `functions/sendMessage.ts` | The critical write path — transactional |
| `functions/toggleReaction.ts` | Race-safe toggle with unique-index fallback |
| `client/ChatApp.tsx` | React UI — `useQuery`, `useRoom`, `useMutation` |

## What this example does NOT do

- **No real magic-code login.** `upsertUser` accepts any email. Delete it
  before shipping; wire up `/api/auth/magic/send` + `/magic/verify`.
- **No private DMs.** Channels can be private but there's no 1:1 DM
  shortcut. Trivial to add: a `Channel` with exactly two memberships.
- **No file uploads.** Use `uploadFile` from `@pylon/react`.
- **No message editing / deletion.** Add an `edit` mutation that checks
  `authorId === ctx.auth.userId` and sets `editedAt`.
- **No threads UI.** The schema supports `parentMessageId`; rendering a
  thread sidebar is 20-30 more lines.
- **No search.** Register the `search` plugin and add a `$search` filter
  on Message — `crates/plugin/src/builtin/search.rs` is FTS5-backed.
- **No webhook integration.** To forward email-to-message, wire an
  action at `/api/webhooks/email_in` using the httpAction pattern.

## Why this exercises the stack

| Feature | Where |
|---|---|
| Change log + WS fan-out | Every `db.useQuery` updates live |
| Tenant scope plugin | `tenantId` auto-stamped; cross-tenant reject |
| Policy engine | 4 declarative `allow` expressions, not code |
| Transactional mutations | `sendMessage` rolls back on any throw |
| Rooms (ephemeral state) | Typing + presence via `useRoom` |
| Unique indexes | Reaction dedup + channel-name uniqueness |
| Paginated queries | `usePaginatedQuery` ready for long histories |
| Session auth | Guest → upgrade → named user |

If it works here, it works anywhere.
