/**
 * `@pylonsync/stripe` — declarative Stripe billing for Pylon apps.
 *
 * Replaces the per-app rewrite of customer creation, checkout
 * session minting, billing portal, webhook signature verification,
 * plan derivation, and subscription state management.
 *
 * Usage (in your app.ts):
 *
 * ```ts
 * import { buildManifest } from "@pylonsync/sdk";
 * import { stripe } from "@pylonsync/stripe";
 *
 * const billing = stripe({
 *   referenceType: "org",
 *   plans: [
 *     { name: "starter", priceId: "price_...", limits: { recordings: 50 } },
 *     { name: "pro",     priceId: "price_...", limits: { recordings: 500 } },
 *     { name: "scale",   priceId: "price_...", limits: { recordings: -1 } },
 *   ],
 *   hooks: {
 *     onSubscriptionActivate: async (ctx, { referenceId, plan }) => {
 *       // analytics, welcome email, etc.
 *     },
 *   },
 * });
 *
 * console.log(JSON.stringify(buildManifest({
 *   name: "myapp",
 *   entities: [Org, User, ...billing.manifest.entities],
 *   actions:  [...billing.manifest.actions],
 *   queries:  [...billing.manifest.queries],
 *   policies: [...billing.manifest.policies],
 * }), null, 2));
 * ```
 *
 * Then create one-line wrapper files in `functions/`:
 *
 * ```ts
 * // functions/createCheckoutSession.ts
 * export { createCheckoutSession as default } from "../billing";
 * ```
 *
 * Where `../billing.ts` is:
 *
 * ```ts
 * // billing.ts
 * import { stripe } from "@pylonsync/stripe";
 * export const billingConfig = stripe({ ... });
 * export const {
 *   createCheckoutSession,
 *   createBillingPortalSession,
 *   cancelSubscription,
 *   restoreSubscription,
 *   stripeWebhook,
 *   ...internals
 * } = billingConfig.handlers;
 * ```
 *
 * Pylon's file-based function loader requires one default export
 * per function file, which is why the per-function wrappers exist.
 * A future SDK version may expose a `plugins: [...]` field on
 * `buildManifest()` that auto-registers handler factories without
 * the wrapper files; until then, the wrapper layer keeps the
 * boilerplate to 1 line per function.
 */

import { buildStripeManifest, type StripeManifestFragment } from "./manifest";
import {
	cancelSubscriptionHandler,
	createBillingPortalSessionHandler,
	createCheckoutSessionHandler,
	internalHandlers,
	restoreSubscriptionHandler,
	stripeWebhookHandler,
} from "./handlers";
import type { StripeConfig } from "./types";

export type {
	StripeConfig,
	StripePlan,
	StripeHooks,
	StripeEvent,
	StripeSubscription,
	StripeInvoice,
	HandlerCtx,
	ReferenceType,
	SubscriptionAction,
} from "./types";
export type { StripeManifestFragment } from "./manifest";
export { StripeError } from "./client";
export { verifyStripeSignature } from "./signature";
export { assertSafeRedirectUrl } from "./safe-url";

export interface StripePlugin {
	/** Manifest fragment to merge into `buildManifest()`. */
	manifest: StripeManifestFragment;
	/**
	 * Handler exports. One per function file: re-export as the
	 * default export of a file under `functions/<name>.ts`. The
	 * internal `_pylonStripe*` handlers live under `internals` and
	 * also need re-exporting (one wrapper per name).
	 */
	handlers: {
		createCheckoutSession: ReturnType<typeof createCheckoutSessionHandler>;
		createBillingPortalSession: ReturnType<
			typeof createBillingPortalSessionHandler
		>;
		cancelSubscription: ReturnType<typeof cancelSubscriptionHandler>;
		restoreSubscription: ReturnType<typeof restoreSubscriptionHandler>;
		stripeWebhook: ReturnType<typeof stripeWebhookHandler>;
	} & Record<string, unknown>;
}

export function stripe(cfg: StripeConfig): StripePlugin {
	return {
		manifest: buildStripeManifest(cfg),
		handlers: {
			createCheckoutSession: createCheckoutSessionHandler(cfg),
			createBillingPortalSession: createBillingPortalSessionHandler(cfg),
			cancelSubscription: cancelSubscriptionHandler(cfg),
			restoreSubscription: restoreSubscriptionHandler(cfg),
			stripeWebhook: stripeWebhookHandler(cfg),
			...internalHandlers(cfg),
		},
	};
}
