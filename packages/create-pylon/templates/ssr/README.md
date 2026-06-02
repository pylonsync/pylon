# __APP_NAME__

A full-stack [Pylon](https://pylonsync.com) app — server-rendered React,
file-based routes, a synced database, and a typed client, served from one
binary on one port. No Next.js, no separate API server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Edit any file under `app/` and save — the page
reloads instantly.

## Layout

```
app.ts            your data model + manifest (entities, functions, policies, routes)
app/              file-based SSR routes — app/page.tsx is "/", app/counter/page.tsx is "/counter"
app/layout.tsx    the root layout wrapping every page (receives url + auth)
app/globals.css   Tailwind entrypoint (compiled by Pylon)
functions/        server functions (query/action) — typed RPC, auto-exposed
```

## Add a route

Drop a file at `app/about/page.tsx` and visit `/about`. Pages receive
`{ url, auth, searchParams }` from the SSR runtime.

## Add data

Edit `app.ts`. Every `entity()` becomes a synced table with a REST +
realtime API and a typed client — no migrations, no resolvers.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
