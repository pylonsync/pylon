import { revenuecat } from "@pylonsync/revenuecat";

/** Entitlement identifier as configured in the RevenueCat dashboard. */
export const PRO_ENTITLEMENT = "pro";

/** Notes a free account can keep. The `createNote` action enforces it. */
export const FREE_NOTE_LIMIT = 10;

// One plugin instance for the whole app. Its manifest fragment is spread
// into app.ts; its handlers are re-exported from functions/ (one file per
// function, because the loader wants one default export per file).
//
// Env (apps/api/.env):
//   REVENUECAT_WEBHOOK_AUTH   the Authorization value you set on the dashboard webhook
//   REVENUECAT_SECRET_KEY     for syncEntitlements (a public SDK key also works)
// Point the dashboard webhook at https://<your-app>/api/fn/revenuecatWebhook.
export const purchases = revenuecat({
  entitlements: [PRO_ENTITLEMENT],
});

export const { revenuecatWebhook, syncEntitlements } = purchases.handlers;
export const { _pylonRcUpsertEntitlement } = purchases.internals;
