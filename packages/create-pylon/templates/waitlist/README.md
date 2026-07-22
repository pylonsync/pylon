# __APP_NAME__

A pre-launch landing page built with [Pylon](https://pylonsync.com), with a
server-rendered marketing page, live signup counter, and private owner
dashboard.

The signup counter updates on every open page as people join the waitlist.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321, submit an email in one tab, and watch the counter
increment in another without refreshing.

## How the realtime works

- `functions/joinWaitlist.ts`: a public **mutation** that validates, lowercases,
  and dedupes the email, then inserts one `Signup` row. The insert fires a
  change event.
- `functions/waitlistCount.ts`: a public **query** the landing page subscribes
  to with `db.useReactiveQuery`. The server records that it read the `Signup`
  table, so every new signup re-runs it and pushes the fresh count to every open
  tab. No polling.
- The counter island (`app/waitlist-hero.tsx`) is wrapped in `<EnsureGuest>`,
  which mints an anonymous session so the live WebSocket can connect.

## Privacy

The `Signup` entity holds visitor emails (PII), so its policy in `app.ts`
**denies every client read and write**. Emails can never be pulled from the
browser. The public page only ever receives an aggregate *count* (a bare
integer); the full list — including emails — is returned only by
`waitlistStats`, which is gated to the owner server-side. A marketing site must
never leak its own customers' emails, and this is how that's guaranteed.

## The owner dashboard

`/dashboard` shows the total, a signups-over-time chart, a searchable list, and
a CSV export — all updating live as people join.

It's single-tenant: set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to
the email you'll sign in with, then create that account at `/login`. Only that
account can see signups; anyone else gets a locked screen.

## Rebrand it

Brand, colors, hero copy, value props, social proof, and FAQ content live in
**`lib/site.config.ts`**. Edit or generate that file to update the page without
changing JSX or CSS.

## Layout

```
app.ts                       data model + manifest (Signup, User, policies, auth)
lib/site.config.ts           ALL business copy + brand + colors (edit this)
lib/owner.ts                 owner-email gate (PYLON_OWNER_EMAIL)
lib/stats.ts                 shared dashboard-stats types
functions/joinWaitlist.ts    public mutation: validate + dedupe + insert
functions/waitlistCount.ts   public reactive query: the live counter
functions/waitlistStats.ts   owner-only reactive query: total + chart + list
app/page.tsx                 the landing page (server-rendered)
app/waitlist-hero.tsx        client island: signup form + live counter
app/login/page.tsx           owner sign-in
app/dashboard/               owner dashboard (auth-gated, live)
app/globals.css              Tailwind entrypoint (compiled by Pylon)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
