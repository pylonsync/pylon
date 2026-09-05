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
	RcEntitlementRow,
	EntitlementChange,
	HandlerCtx,
} from "./types";
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

/**
 * Client-side helper: is `entitlement` active in a list of rows the app
 * read through sync? Treats a row with a past `expiresAt` as inactive even
 * if the webhook has not delivered the expiration yet.
 */
export function hasEntitlement(
	rows: ReadonlyArray<{ entitlement: string; status: string; expiresAt?: string | null }>,
	entitlement: string,
	now: number = Date.now(),
): boolean {
	return rows.some(
		(r) =>
			r.entitlement === entitlement &&
			r.status === "active" &&
			(r.expiresAt == null || new Date(r.expiresAt).getTime() > now),
	);
}
