# __APP_NAME__ API

The Pylon backend for the __APP_NAME__ mobile app. One process serves the
data API, realtime sync, auth, and the RevenueCat webhook.

```
app.ts                 User + Note + RcEntitlement (from @pylonsync/revenuecat)
functions/createNote   server-enforced free-tier cap, then insert
functions/revenuecatWebhook, syncEntitlements, _pylonRcUpsertEntitlement
lib/purchases.ts       the RevenueCat plugin instance + FREE_NOTE_LIMIT
```

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
and point the RevenueCat webhook at `<url>/api/fn/revenuecatWebhook`.
