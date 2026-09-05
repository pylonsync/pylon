/**
 * Manifest fragment: the entitlement entity, its read policy, and the
 * action declarations. App code spreads these into `buildManifest()`.
 */

import {
	type ActionDefinition,
	type EntityDefinition,
	type PolicyDefinition,
	action,
	entity,
	field,
	policy,
} from "@pylonsync/sdk";

import type { RevenueCatConfig } from "./types";

export interface RevenueCatManifestFragment {
	entities: EntityDefinition[];
	actions: ActionDefinition[];
	policies: PolicyDefinition[];
}

export function entitlementEntityName(cfg: RevenueCatConfig): string {
	return cfg.entities?.entitlement ?? "RcEntitlement";
}

/**
 * ```ts
 * const purchases = revenuecat({ entitlements: ["pro"] });
 * buildManifest({
 *   entities: [User, ...purchases.manifest.entities],
 *   actions:  [...fns.actions, ...purchases.manifest.actions],
 *   policies: [...purchases.manifest.policies],
 * });
 * ```
 */
export function buildRevenueCatManifest(cfg: RevenueCatConfig): RevenueCatManifestFragment {
	const name = entitlementEntityName(cfg);

	// One row per (user, entitlement). Written only by the webhook and the
	// server-side sync; clients read their own rows through sync, so a
	// purchase on one device unlocks every device.
	const Entitlement = entity(
		name,
		{
			userId: field.id("User"),
			entitlement: field.string(),
			productId: field.string(),
			// "active" | "expired"
			status: field.string(),
			// app_store | play_store | stripe | promotional | sdk_sync
			store: field.string(),
			environment: field.string().optional(),
			expiresAt: field.string().optional(),
			updatedAt: field.string(),
		},
		{
			indexes: [
				{ name: "by_user_entitlement", fields: ["userId", "entitlement"], unique: true },
			],
		},
	);

	const readOwn = policy({
		name: `${name.toLowerCase()}_read_own`,
		entity: name,
		allowRead: "auth.userId == data.userId",
		allowInsert: "false",
		allowUpdate: "false",
		allowDelete: "false",
	});

	return {
		entities: [Entitlement],
		policies: [readOwn],
		actions: [
			// Public by handler: RevenueCat posts anonymously; the shared
			// Authorization header is the auth boundary.
			action("revenuecatWebhook"),
			// Signed-in user: re-read their entitlements from RevenueCat and
			// write the rows. Called after a purchase or restore.
			action("syncEntitlements"),
		],
	};
}
