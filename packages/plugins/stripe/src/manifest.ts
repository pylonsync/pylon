/**
 * Manifest fragments — entities + action declarations the plugin
 * contributes to an app's manifest. App code spreads these into
 * its `buildManifest({ entities, actions })` call.
 */

import {
	type ActionDefinition,
	type EntityDefinition,
	type PolicyDefinition,
	type QueryDefinition,
	action,
	entity,
	field,
	policy,
	query,
} from "@pylonsync/sdk";

import type { StripeConfig } from "./types";

export interface StripeManifestFragment {
	entities: EntityDefinition[];
	actions: ActionDefinition[];
	queries: QueryDefinition[];
	policies: PolicyDefinition[];
}

/**
 * Build the manifest fragment for the Stripe plugin. App composes
 * this into its top-level manifest:
 *
 *   ```ts
 *   const billing = stripe({ referenceType: "org", plans: [...] });
 *
 *   buildManifest({
 *     entities: [Org, User, ...billing.manifest.entities],
 *     actions:  [...billing.manifest.actions],
 *     queries:  [...billing.manifest.queries],
 *     policies: [...billing.manifest.policies],
 *   });
 *   ```
 */
export function buildStripeManifest(
	cfg: StripeConfig,
): StripeManifestFragment {
	const subscriptionEntity = cfg.entities?.subscription ?? "StripeSubscription";

	// Single source of truth for billing state. One row per
	// referenceId — when a customer upgrades/cancels the row updates
	// in place. Indexed by stripeSubscriptionId for webhook lookups.
	const Subscription = entity(subscriptionEntity, {
		referenceId: field.string(),
		stripeCustomerId: field.string(),
		stripeSubscriptionId: field.string().unique(),
		plan: field.string(),
		status: field.string(),
		seats: field.number().optional(),
		currentPeriodEnd: field.string().optional(),
		cancelAtPeriodEnd: field.boolean().optional(),
		canceledAt: field.string().optional(),
		trialEnd: field.string().optional(),
		annual: field.boolean().optional(),
		limits: field.string().optional(),
		createdAt: field.string(),
		updatedAt: field.string(),
	});

	// Policy: subscriptions are tenant-scoped via referenceId. App
	// auth context decides whether that referenceId is a user or
	// org — the policy treats it opaquely. When referenceType is
	// "org", the active tenant maps 1:1; when "user", the auth
	// userId maps 1:1.
	const subscriptionPolicy = policy({
		name: `${subscriptionEntity.toLowerCase()}_read`,
		entity: subscriptionEntity,
		allowRead:
			cfg.referenceType === "org"
				? "auth.tenantId == data.referenceId"
				: "auth.userId == data.referenceId",
		allowInsert: "false",
		allowUpdate: "false",
		allowDelete: "false",
	});

	return {
		entities: [Subscription],
		policies: [subscriptionPolicy],
		queries: [
			query("listSubscriptions"),
			query("getSubscription", {
				input: [{ name: "referenceId", type: "string" }],
			}),
		],
		actions: [
			action("createCheckoutSession", {
				input: [
					{ name: "plan", type: "string" },
					{ name: "referenceId", type: "string", optional: true },
					{ name: "successUrl", type: "string" },
					{ name: "cancelUrl", type: "string" },
					{ name: "annual", type: "bool", optional: true },
					{ name: "seats", type: "int", optional: true },
				],
			}),
			action("createBillingPortalSession", {
				input: [
					{ name: "referenceId", type: "string", optional: true },
					{ name: "returnUrl", type: "string" },
				],
			}),
			action("cancelSubscription", {
				input: [
					{ name: "referenceId", type: "string", optional: true },
					{ name: "scheduleAtPeriodEnd", type: "bool", optional: true },
				],
			}),
			action("restoreSubscription", {
				input: [{ name: "referenceId", type: "string", optional: true }],
			}),
			action("stripeWebhook"),
		],
	};
}
