# __APP_NAME__

A full-stack [Pylon](https://pylonsync.com) app — a server-rendered homepage,
email/password auth, and a live client dashboard over a synced database, all
served from one binary on one port. No Next.js, no separate API server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Sign up, and your notes dashboard updates live
(open a second tab to watch writes sync). Edit any file under `app/` and save —
the page reloads instantly.

## Layout

```
app.ts                 data model + manifest (entities, policies, auth, routes)
app/page.tsx           "/" — the server-rendered, auth-aware homepage
app/login,signup/      email/password forms (POST /api/auth/password/*)
app/dashboard/         "/dashboard" — authed; server-gated, live notes + sign out
app/auth-form.tsx      shared client island for the login/signup forms
app/layout.tsx         root layout wrapping every page (auth-aware nav)
app/globals.css        Tailwind entrypoint (compiled by Pylon)
functions/             server functions (query/mutation/action) — typed RPC
```

## How auth works

Email/password is built in. `/login` and `/signup` call
`/api/auth/password/*`; on success the server sets an **HttpOnly session
cookie** (no token in JS-readable storage). `/dashboard` reads `auth` during
the server render and redirects anonymous visitors to `/login` — a real 3xx
before any HTML, so there's no flash and it works with JS off. The sync engine
authenticates with the same cookie.

## Add a route

Drop a file at `app/about/page.tsx` and visit `/about`. Pages receive
`{ url, params, searchParams, auth, response, serverData }` from the SSR
runtime — all typed via `PageProps` from `@pylonsync/react`.

## Add data

Edit `app.ts`. Every `entity()` becomes a synced table with a REST +
realtime API and a typed client — no migrations, no resolvers.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
