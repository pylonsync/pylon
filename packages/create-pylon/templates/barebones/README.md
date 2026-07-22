# __APP_NAME__

The smallest useful [Pylon](https://pylonsync.com) app has one entity, a live
list, and an optimistic create. Pylon server-renders the page and serves the
synced backend from one process.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and add an item. It appears optimistically and syncs
to a second tab. Editing a file under `app/` reloads the page.

## Layout

```
app.ts              data model + manifest (the Item entity, its policy, auth)
app/page.tsx        "/" — the server-rendered page
app/items-client.tsx client island: guest session + live, optimistic list
app/layout.tsx      root layout wrapping every page
app/globals.css     Tailwind entrypoint (compiled by Pylon)
functions/          server functions (query/action) — add them when you need them
```

## Grow it

- **Add a field:** edit the `Item` entity in `app.ts` — the typed client and
  REST/realtime API follow, no migration.
- **Add an entity:** declare another `entity()` + a `policy()`; an entity with
  no policy is denied to clients by default.
- **Add a route:** drop `app/about/page.tsx` and visit `/about`.
- **Require accounts:** email/password is built in — enable it against a
  `User` entity and swap `<EnsureGuest>` for `<SignedIn>` / `<SignedOut>` from
  `@pylonsync/client`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
