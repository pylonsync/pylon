# __APP_NAME__

A personal-brand / creator site built with [Pylon](https://pylonsync.com) — a
server-rendered landing page with a **live newsletter subscriber counter** and a
private owner dashboard, all from one binary on one port. No Next.js, no
separate API server.

The realtime point: the subscriber count ticks up for everyone with the page
open the instant someone subscribes.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Then **open a second tab**, subscribe in one, and
watch the counter increment in the other — with no refresh.

## How the realtime works

- `functions/subscribe.ts` — a public **mutation** that validates, lowercases,
  and dedupes the email, inserts one `Subscriber` row, and bumps a public,
  PII-free `SubscriberCount` row.
- `app/newsletter-signup.tsx` subscribes to `SubscriberCount` with
  `db.useQuery`, so the live count syncs to every open tab. No polling.

## Privacy — read this

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

Everything lives in **`lib/site.config.ts`** — your name, colors, bio,
offerings, testimonials, newsletter copy, links. Edit that one file (or have a
generator produce it) and the whole page re-themes.

## Layout

```
app.ts                       Subscriber + SubscriberCount + User + policies
lib/site.config.ts           ALL copy + brand + offerings + newsletter (edit this)
functions/subscribe.ts       public mutation: validate + dedupe + count
functions/subscriberStats.ts owner-only query: subscribers + emails
app/page.tsx                 the landing page (server-rendered)
app/newsletter-signup.tsx    client island: signup form + live counter
app/dashboard/               owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
