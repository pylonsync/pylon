# From scaffold to the App Store

The order below is the shortest path that App Review and Google Play accept
on the first try. Each step is one command or one dashboard page.

## 1. Backend live

```bash
cd apps/api && pylon deploy
pylon secrets set PYLON_APPLE_NATIVE_CLIENT_IDS=<bundle id> REVENUECAT_WEBHOOK_AUTH=<random> REVENUECAT_SECRET_KEY=<key>
```

Put the deployed URL in `apps/expo/eas.json` (`EXPO_PUBLIC_PYLON_BASE_URL`
under `preview` and `production`).

## 2. Identifiers

- Pick a bundle id (`com.yourco.app`) and set `APP_BUNDLE_ID` in `eas.json`
  env for every profile, or export it before building.
- `eas init` prints the project id; set `EAS_PROJECT_ID` the same way.
- Apple: an App ID with the Sign in with Apple capability. EAS creates it on
  the first `eas build` when you let it manage credentials.
- Google: OAuth client ids (iOS + Android + Web) in Google Cloud. Put the
  web and iOS ids in `apps/expo/.env` and both ids in
  `PYLON_GOOGLE_NATIVE_CLIENT_IDS` on the backend.

## 3. Products

1. App Store Connect → the app → Subscriptions: one group, monthly + annual
   (annual with a 7-day introductory free trial converts best).
2. Google Play Console → Monetize → Subscriptions: the same two products.
3. RevenueCat → Project: add both apps, import the products, make an
   entitlement `pro`, and an offering "default" with a monthly and an annual
   package. Copy the iOS and Android public SDK keys to `apps/expo/.env`.
4. RevenueCat → Integrations → Webhooks: URL `<backend>/api/fn/revenuecatWebhook`,
   Authorization header = the `REVENUECAT_WEBHOOK_AUTH` value.

## 4. Build

```bash
eas build --profile development --platform ios   # dev client for the simulator
eas build --profile preview --platform all       # TestFlight / internal testing
eas build --profile production --platform all    # store build
```

Test the purchase on TestFlight with a sandbox Apple ID before submitting.

## 5. Store listing

Both stores require, and reviewers check:

- Privacy policy URL and terms URL (`EXPO_PUBLIC_PRIVACY_URL`, `EXPO_PUBLIC_TERMS_URL`).
  The app shows them in Settings and under the paywall.
- Account deletion inside the app (Settings → Delete account). Present.
- Sign in with Apple when any other third-party sign-in is offered. Present.
- Subscription terms next to the purchase button (price, period, renewal). Present.
- Restore purchases. Present in the paywall and Settings.
- App Privacy answers: this template stores email (account), user content
  (notes), and purchase history (via RevenueCat). No tracking.
- Screenshots: 6.7" and 6.1" iPhone; 7" and 10" tablets are optional when
  `supportsTablet` is false.
- A demo account for the reviewer: create one with an email code and put
  the address plus "use the email code sent to this address" in Review
  Notes, or turn on `PYLON_DEV_MODE`-style test codes on a staging backend.

## 6. Submit

```bash
eas submit --platform ios --latest
eas submit --platform android --latest
```

Fill `submit.production.ios.ascAppId` in `eas.json` with the App Store
Connect app id first.

## After launch

- The `track()` calls in `src/analytics.ts` already name the funnel:
  onboarding → paywall_shown → purchase_completed. Wire them to your
  analytics SDK to see conversion per step.
- Prices, trials, and which packages appear are RevenueCat offering
  settings; change them without shipping an update.
