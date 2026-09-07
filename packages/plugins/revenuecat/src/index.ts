/**
 * `@pylonsync/revenuecat` — native in-app purchases for Pylon apps.
 *
 * RevenueCat handles the App Store / Play Billing side (products,
 * receipts, renewals, paywalls). This plugin turns its webhook events
 * into an `RcEntitlement` row per (user, entitlement) that syncs to every
 * device like any other row, so a purchase on the phone unlocks the
 * tablet and the web app without a second receipt check.
 *
 * ```ts
 * // lib/purchases.ts
 * import { revenuecat } from "@pylonsync/revenuecat";
 * export const purchases = revenuecat({ entitlements: ["pro"] });
 * export const { revenuecatWebhook, syncEntitlements } = purchases.handlers;
 * export const { _pylonRcUpsertEntitlement } = purchases.internals;
 *
 * // functions/revenuecatWebhook.ts
 * export { revenuecatWebhook as default } from "../lib/purchases";
 * // functions/syncEntitlements.ts
 * export { syncEntitlements as default } from "../lib/purchases";
 * // functions/_pylonRcUpsertEntitlement.ts
 * export { _pylonRcUpsertEntitlement as default } from "../lib/purchases";
 * ```
 *
 * Env:
 *   REVENUECAT_WEBHOOK_AUTH   the Authorization value set on the dashboard webhook
 *   REVENUECAT_SECRET_KEY     (or REVENUECAT_PUBLIC_KEY) for the subscriber read in syncEntitlements
 *
 * Point the RevenueCat webhook at `POST /api/fn/revenuecatWebhook`.
 *
 * App code reads entitlements through sync and gates with
 * `hasEntitlement` from `@pylonsync/revenuecat/client`. Do not import
 * this root module from a browser or React Native bundle: it pulls in
 * `@pylonsync/functions`.
 */

import { buildRevenueCatManifest, type RevenueCatManifestFragment } from "./manifest";
import {
	INTERNAL_UPSERT,
	revenuecatWebhookHandler,
	syncEntitlementsHandler,
	upsertEntitlementHandler,
} from "./handlers";
import type { RevenueCatConfig } from "./types";

export type {
	RevenueCatConfig,
	RevenueCatHooks,
	RevenueCatEvent,
	EntitlementChange,
	HandlerCtx,
} from "./types";
export type { RcEntitlementRow } from "./client";
export type { RevenueCatManifestFragment } from "./manifest";
export { entitlementEntityName } from "./manifest";
export { statusForEvent, entitlementIdsOf, webhookAuthorized } from "./handlers/webhook";
export { entitlementsFromSubscriber } from "./handlers/sync";

export interface RevenueCatPlugin {
	manifest: RevenueCatManifestFragment;
	handlers: {
		revenuecatWebhook: ReturnType<typeof revenuecatWebhookHandler>;
		syncEntitlements: ReturnType<typeof syncEntitlementsHandler>;
	};
	internals: Record<string, ReturnType<typeof upsertEntitlementHandler>>;
}

export function revenuecat(cfg: RevenueCatConfig): RevenueCatPlugin {
	return {
		manifest: buildRevenueCatManifest(cfg),
		handlers: {
			revenuecatWebhook: revenuecatWebhookHandler(cfg),
			syncEntitlements: syncEntitlementsHandler(cfg),
		},
		internals: {
			[INTERNAL_UPSERT]: upsertEntitlementHandler(cfg),
		},
	};
}

// `hasEntitlement` also ships from `@pylonsync/revenuecat/client`, which
// carries no server imports. App code (browser, React Native) must import
// from there; this root re-export keeps server-side callers working.
export { hasEntitlement } from "./client";
