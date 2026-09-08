# __APP_NAME__ API

The Pylon backend for the __APP_NAME__ mobile app. One process serves the
data API, realtime sync, auth, and the RevenueCat webhook.

```
app.ts                 User + Note + RcEntitlement (from @pylonsync/revenuecat)
functions/createNote   server-enforced free-tier cap, then insert
functions/deleteMyData the user's rows, run by DELETE /api/auth/account
functions/revenuecatWebhook, syncEntitlements, _pylonRcUpsertEntitlement
lib/purchases.ts       the RevenueCat plugin instance + FREE_NOTE_LIMIT
lib/site.ts            everything the website says — edit this one file
app/(site)/            landing page, /support, /privacy, /terms
```

## The website

This one server answers the app's API calls and serves the public site, so
`/privacy` and `/terms` are live on the same host the moment you deploy. Both
stores refuse a submission without a reachable privacy policy URL, and the app
links to these pages from Settings and from the paywall.

Edit `lib/site.ts` for the name, the copy, store links, and your company
details. A banner sits on every page until those details are filled in,
because the legal text ships as a draft: it describes what this app actually
does, which makes it a real starting point, but a lawyer should read it before
you submit.

`bun run dev` serves the site at http://localhost:4321 alongside the API.

## Run

```bash
bun run dev      # http://localhost:4321 — the Expo app points here in dev
```

Magic codes print to this console in dev. Native sign-in needs the ids in
`.env.example`.

## Deploy

```bash
pylon deploy
pylon secrets set REVENUECAT_WEBHOOK_AUTH=... REVENUECAT_SECRET_KEY=... \
  PYLON_APPLE_NATIVE_CLIENT_IDS=com.example.__APP_NAME_SNAKE__
```

Then set `EXPO_PUBLIC_PYLON_BASE_URL` in `apps/expo/.env` to the deployed URL
and point the RevenueCat webhook at `<url>/api/fn/revenuecatWebhook`. Put the
same URL in `lib/site.ts` as `url`, and give App Store Connect `<url>/privacy`
and `<url>/support`.
