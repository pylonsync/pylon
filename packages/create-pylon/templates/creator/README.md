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
  and dedupes the email, inserts one `Subscriber` row, and bumps a public,
  PII-free `SubscriberCount` row.
- `app/newsletter-signup.tsx` subscribes to `SubscriberCount` with
  `db.useQuery`, so the live count syncs to every open tab. No polling.

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
