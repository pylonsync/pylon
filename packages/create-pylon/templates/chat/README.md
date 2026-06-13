# __APP_NAME__

A realtime chat room on [Pylon](https://pylonsync.com) — a live, shared message
room served over one synced backend. One binary, one port. No Next.js, no
separate API server, no realtime sidecar.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321, then open a **second tab** — type in one and the
message appears in the other instantly. Edit any file under `app/` and save —
the page reloads.

## Layout

```
app.ts              data model: the Message entity + room policy + auth
app/page.tsx        "/" — the server-rendered page
app/chat-client.tsx client island: guest session + live messages, optimistic send
app/layout.tsx      full-height root layout
app/globals.css     Tailwind entrypoint (compiled by Pylon)
```

## How it works

No login wall: `app/chat-client.tsx` wraps the room in `<EnsureGuest>`, which
mints a guest session so anyone can chat. The room is **public-read** (everyone
sees every message — that's a chat room), and `authorId: field.owner()` stamps
the sender server-side so a message can't be spoofed. `db.useQuery("Message")`
is a **live subscription** — new messages render the instant they're sent, in
any tab, with no polling and no extra code.

## Grow it

- **Multiple rooms:** add a `Room` entity and a `roomId` field on `Message`,
  then filter `db.useQuery` by the selected room.
- **Presence ("who's here"):** use a presence channel (`ctx.connections.*`) to
  track connected users.
- **Real accounts:** email/password is built in — swap `<EnsureGuest>` for
  `<SignedIn>` / `<SignedOut>` from `@pylonsync/client`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
