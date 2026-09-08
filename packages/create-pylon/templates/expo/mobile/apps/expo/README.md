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
src/links.ts               privacy / terms / support URLs, served by apps/api
STORE.md                   the submission checklist
```

The public website (landing page, `/privacy`, `/terms`, `/support`) is served
by the backend in `apps/api`, on the same host as the API. The links in
Settings and under the paywall point there with no configuration, which is
what makes the store requirement satisfiable from a fresh scaffold. Edit its
copy in `apps/api/lib/site.ts`.

## Run

```bash
cp .env.example .env
bun run dev          # Expo Go: everything except native sign-in and purchases
eas build --profile development --platform ios && bun run dev   # dev build: everything
```

The backend must be running (`cd ../api && bun run dev`, port 4321) or
deployed (`EXPO_PUBLIC_PYLON_BASE_URL` in `.env`). An Android emulator
reaches the host machine at `http://10.0.2.2:4321`; `src/pylon.ts` uses
that when the variable is unset.

`bun run check` typechecks. `bun run check:bundle` exports the iOS and
Android bundles without a device; run it after changing dependencies.

Adding an Expo native module (a package with `ios/` or `android/`) needs a
new development build (`eas build --profile development`). The Metro
server can keep running.

## Replace the demo

The Notes screens are placeholders. When you replace them, these files
also carry Notes-specific copy or logic:

- `app/(onboarding)/welcome.tsx`: the three slides.
- `app/(auth)/sign-in.tsx`: the line under the title.
- `app/paywall.tsx`: `BENEFITS` and the reason text.
- `app/(tabs)/settings.tsx`: the delete-account confirmation.
- `app/(tabs)/index.tsx`: the list, `FREE_LIMIT`, and the sign-in nudge.
- `app.config.ts`: `name`, `slug`, `scheme`, and the icons in `assets/`.
- `apps/api/lib/site.ts`: the website copy, store links, and the company
  details the privacy policy and terms need.
- `apps/api/functions/deleteMyData.ts`: delete every entity that stores
  user data, or account deletion leaves rows behind.
- `apps/api/functions/createNote.ts`: the free-tier cap.

## Ship

See `STORE.md`.
