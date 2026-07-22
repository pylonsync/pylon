# __APP_NAME__

A full-stack [Pylon](https://pylonsync.com) todo app with a server-rendered page
and a live, optimistic, per-user list over a synced database.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and add a todo. It appears optimistically and syncs
to a second tab. Editing a file under `app/` reloads the page.

## Layout

```
app.ts              data model + manifest (entities, policies, auth, routes)
app/page.tsx        "/" — the server-rendered page (heading + intro)
app/todo-app.tsx    client island: guest session + live, optimistic todo list
app/layout.tsx      root layout wrapping every page
app/globals.css     Tailwind entrypoint (compiled by Pylon)
functions/          server functions (query/action) — typed RPC, if you need them
```

## How it works

No login wall: `app/todo-app.tsx` wraps the list in `<EnsureGuest>`, which
POSTs `/api/auth/guest` on first load so every visitor implicitly becomes their
own user. Todos are private per browser — the `todo_access` policy in `app.ts`
gates every read and write to the owner, and `userId: field.owner()` stamps the
session's id server-side so the optimistic `db.insert("Todo", { title })` can't
be spoofed. `db.useQuery("Todo")` is a live subscription; `db.insert` /
`db.update` / `db.delete` are optimistic.

To require real accounts instead, enable email/password (built in, against a
`User` entity) and swap `<EnsureGuest>` for `<SignedIn>` / `<SignedOut>` from
`@pylonsync/client`.

## Add a route

Drop a file at `app/about/page.tsx` and visit `/about`. Pages receive
`{ url, params, searchParams, auth, response, serverData }` from the SSR
runtime — all typed via `PageProps` from `@pylonsync/react`.

## Add data

Edit `app.ts`. Every `entity()` becomes a synced table with a REST + realtime
API and a typed client — no migrations, no resolvers.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
