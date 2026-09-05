# __APP_NAME__ (Expo)

The mobile app. Expo Router, guest-first onboarding, Sign in with Apple /
Google / email code, a RevenueCat paywall, and an offline-capable synced
list, wired to the Pylon backend in `apps/api`.

```
app/_layout.tsx            session-driven route guard
app/(onboarding)/welcome   three slides → guest session → paywall (once) → app
app/(auth)/sign-in, verify Apple, Google, email code
app/paywall                RevenueCat offering; annual preselected; restore
app/(tabs)/index           the Notes list (free cap enforced server-side)
app/(tabs)/settings        account, subscription, legal, delete account
src/session.tsx            boot + state machine
src/purchases.ts           RevenueCat wrapper, safe in Expo Go
src/entitlements.ts        usePro() from the synced RcEntitlement rows
src/analytics.ts           funnel events; wire to your SDK in one place
STORE.md                   the submission checklist
```

## Run

```bash
cp .env.example .env
bun run dev          # Expo Go: everything except native sign-in and purchases
eas build --profile development --platform ios && bun run dev   # dev build: everything
```

The backend must be running (`cd ../api && bun run dev`) or deployed
(`EXPO_PUBLIC_PYLON_BASE_URL` in `.env`).

## Ship

See `STORE.md`.
