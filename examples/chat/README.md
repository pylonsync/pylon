# Chat — pylon example

A Slack-style chat app in one manifest + six functions + one React file.

Demonstrates:

- **Live sync**: messages appear in every connected client instantly. No
  explicit push code; the change log fan-out handles it.
- **Rooms API**: `useRoom("channel:<id>")` gives you per-channel presence
  ("3 others here") and typing indicators ("Alice is typing…").
- **Tenant isolation**: `Workspace` is the tenant. Every scoped entity has
  a `tenantId` field; the `tenant_scope` plugin stamps it automatically on
  inserts and rejects cross-tenant reads/writes. Functions do not need to
  call `.where({ tenantId })`.
- **Transactional mutations**: `sendMessage.ts` is atomic: the channel
  lookup, membership check, and message insert all roll back together if
  any step throws.
- **Optimistic UI**: `useMutation("sendMessage")` clears the input
  instantly; the sync engine reconciles when the server confirms.
- **Reactions with racing double-tap**: `toggleReaction.ts` handles
  concurrent "+👍" clicks safely via the `(messageId, userId, emoji)`
  unique index.
- **Read markers**: `markChannelRead.ts` upserts one marker per
  (user, channel); the client compares `lastReadAt` against message
  timestamps to render unread counts.

## Run it

One terminal:

```sh
cd examples/chat
bun dev
```

That starts Pylon on :4321 AND auto-spawns the Vite dev server in
`web/` on :5173. Open two browser windows at `http://localhost:5173`,
sign in as different emails in each, and watch messages land live.

`/api/*` requests from the React app are proxied to Pylon by Vite
(see `web/vite.config.ts`).

Studio at `http://localhost:4321/studio` lets you inspect rows directly
(requires admin token in non-dev mode).

## What to read first

| File | Why |
|---|---|
| `pylon.manifest.json` | Data model — 7 entities, 4 policies |
| `functions/sendMessage.ts` | The critical write path — transactional |
| `functions/toggleReaction.ts` | Race-safe toggle with unique-index fallback |
| `web/src/ChatApp.tsx` | React UI — `useQuery`, `useRoom`, `useMutation` |

## Out of scope

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
