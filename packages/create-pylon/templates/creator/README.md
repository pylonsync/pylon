# __APP_NAME__

A personal brand or creator site built with [Pylon](https://pylonsync.com). It
combines a server-rendered landing page, live newsletter subscriber count, and
private owner dashboard in one server.

The subscriber count updates on every open page when someone subscribes.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321, then subscribe in one tab and watch the counter
increment in another without refreshing.

## How the realtime works

- `functions/subscribe.ts`: a public **mutation** that validates, lowercases,
  and dedupes the email, inserts one `Subscriber` row, and recounts into the
  public, PII-free `SubscriberCount` row.
- `app/(marketing)/newsletter-signup.tsx` reads `SubscriberCount` with
  `db.useQuery`, so the live count syncs to every open tab. No polling.

The counter is the `Subscriber` table's row count, and it is hidden while the
list is empty. `newsletter.importedCount` in `lib/site.config.ts` adds
subscribers you are moving over from another tool. It ships as 0.

## Demo data

In `pylon dev`, an empty list gets about 180 fictional subscribers from the last
30 days, so the counter, chart, and list have data (`functions/seedDemo.ts`,
`lib/demo.ts`). Seeding runs only when `PYLON_DEMO_DATA` is on, or when it is unset and the app runs under `pylon dev`, and only while the `Subscriber` table is
empty. `pylon start`, Docker, and Pylon Cloud deploys do not seed, because only `pylon dev` sets `PYLON_DEV_WATCH_DIR`. Set `PYLON_DEMO_DATA=0`
to start empty in development. Delete both files once you have real subscribers.

## Privacy

The `Subscriber` entity holds reader emails (PII), so its policy in `app.ts`
**denies every client read and write**. The public page only ever reads the
aggregate `SubscriberCount` (a bare integer); the full list — with emails —
comes back only through `subscriberStats`, gated to the owner server-side.

## The owner dashboard

`/dashboard` shows total subscribers, a growth chart, a searchable list, and CSV
export — updating live as people subscribe.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`.

## Rebrand it

Your name, colors, bio, offerings, testimonials, newsletter copy, and links
live in **`lib/site.config.ts`**. Edit that file, or generate it, to re-theme
the page.

The tab icon is `app/icon.svg` (with `public/favicon.ico` for browsers that
request `/favicon.ico`). Both carry this brand's letter and colors; replace
them when you rename the brand.

## Layout

```
app.ts                       Subscriber + SubscriberCount + User + policies
lib/site.config.ts           ALL copy + brand + offerings + newsletter (edit this)
functions/subscribe.ts       public mutation: validate + dedupe + count
functions/subscriberStats.ts owner-only query: subscribers + emails
functions/seedDemo.ts        dev-only demo subscribers (see Demo data)
lib/stats.ts                 dashboard types + the public count
lib/demo.ts                  demo subscribers + the dev-only gate
app/(marketing)/page.tsx     the landing page (server-rendered)
app/(marketing)/newsletter-signup.tsx  client island: signup form + live counter
app/dashboard/               owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
