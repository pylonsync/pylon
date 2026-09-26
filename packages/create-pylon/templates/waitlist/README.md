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
  and dedupes the email, inserts one `Signup` row, and rewrites the PII-free
  `WaitlistStat` row with the new count.
- The landing page reads `WaitlistStat` with `db.useQuery`. The row syncs to
  every open tab, so the counter moves as soon as anyone joins. No polling.
- The counter island (`app/(marketing)/waitlist-hero.tsx`) is wrapped in
  `<EnsureGuest>`, which mints an anonymous session so the live connection can
  open.

The counter is the `Signup` table's row count. `counter.importedCount` in
`lib/site.config.ts` adds signups you collected somewhere else before this site
(an old form, a spreadsheet). It ships as 0.

## Demo data

In `pylon dev`, an empty waitlist gets about 180 fictional signups from the last
30 days, so the counter, chart, and list have data (`functions/seedDemo.ts`,
`lib/demo.ts`). Seeding runs only when `PYLON_DEMO_DATA` is on, or when it is unset and the app runs under `pylon dev`, and only while the `Signup` table is empty. `pylon start`, Docker, and Pylon Cloud deploys do not seed, because only `pylon dev` sets `PYLON_DEV_WATCH_DIR`. Set `PYLON_DEMO_DATA=0` to start
empty in development. Delete both files once you have real signups.

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

Brand, colors, hero copy, the launch date, value props, and FAQ content live in
**`lib/site.config.ts`**. Edit or generate that file to update the page without
changing JSX or CSS.

The tab icon is `app/icon.svg` (with `public/favicon.ico` for browsers that
request `/favicon.ico`). Both carry this brand's letter and colors; replace
them when you rename the brand.

## Layout

```
app.ts                       data model + manifest (Signup, User, policies, auth)
lib/site.config.ts           ALL business copy + brand + colors (edit this)
lib/owner.ts                 owner-email gate (PYLON_OWNER_EMAIL)
lib/stats.ts                 dashboard types + the public count
lib/launch.ts                the launch line from hero.launchDate
lib/demo.ts                  demo signups + the dev-only gate
functions/joinWaitlist.ts    public mutation: validate + dedupe + insert
functions/seedDemo.ts        dev-only demo signups (see Demo data)
functions/waitlistStats.ts   owner-only reactive query: total + chart + list
app/(marketing)/page.tsx     the landing page (server-rendered)
app/(marketing)/waitlist-hero.tsx  client island: signup form + live counter
app/login/page.tsx           owner sign-in
app/dashboard/               owner dashboard (auth-gated, live)
app/globals.css              Tailwind entrypoint (compiled by Pylon)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
