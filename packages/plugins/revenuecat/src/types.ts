/**
 * Public type surface for `@pylonsync/revenuecat`.
 */

export interface RevenueCatConfig {
	/**
	 * Entitlement identifiers the app gates on, as configured in the
	 * RevenueCat dashboard (for example `["pro"]`). Unknown identifiers on
	 * a webhook are still stored; this list documents what the app uses.
	 */
	entitlements: string[];
	/**
	 * The shared secret RevenueCat sends as the `Authorization` header on
	 * every webhook (set the same value in the dashboard). Resolved from
	 * `ctx.env.REVENUECAT_WEBHOOK_AUTH` unless overridden. When unset the
	 * webhook answers 403, so a fresh deploy cannot be fed forged
	 * entitlements.
	 */
	getWebhookAuth?: (ctx: HandlerCtx) => string | undefined;
	/**
	 * Key for `GET https://api.revenuecat.com/v1/subscribers/<id>`, used by
	 * `syncEntitlements` to re-read a user's entitlements after a purchase
	 * or restore. Resolved from `ctx.env.REVENUECAT_SECRET_KEY`, then
	 * `ctx.env.REVENUECAT_PUBLIC_KEY` (the public SDK key is enough for
	 * that read), unless overridden.
	 */
	getApiKey?: (ctx: HandlerCtx) => string | undefined;
	/** Entity name override. Default `RcEntitlement`. */
	entities?: { entitlement?: string };
	hooks?: RevenueCatHooks;
}

export interface EntitlementChange {
	userId: string;
	entitlement: string;
	productId: string;
	store: string;
}

export interface RevenueCatHooks {
	/** An entitlement became active for a user (purchase, renewal, restore). */
	onEntitlementActive?: (ctx: HandlerCtx, info: EntitlementChange) => Promise<void> | void;
	/** An entitlement ended (expiration, refund, or a billing issue past grace). */
	onEntitlementExpired?: (ctx: HandlerCtx, info: EntitlementChange) => Promise<void> | void;
	/** Every webhook event, after the rows were written. */
	onEvent?: (ctx: HandlerCtx, event: RevenueCatEvent) => Promise<void> | void;
}

/**
 * The subset of a RevenueCat webhook event this plugin reads.
 * https://www.revenuecat.com/docs/integrations/webhooks/event-types-and-fields
 */
export interface RevenueCatEvent {
	type: string;
	app_user_id?: string;
	original_app_user_id?: string;
	/** Every id RevenueCat has aliased for this subscriber. */
	aliases?: string[];
	product_id?: string;
	entitlement_id?: string | null;
	entitlement_ids?: string[] | null;
	store?: string;
	environment?: string;
	expiration_at_ms?: number | null;
	purchased_at_ms?: number | null;
	period_type?: string;
	cancel_reason?: string;
	new_product_id?: string;
	transaction_id?: string;
	id?: string;
}

/** One row of the entitlement entity. */
export interface RcEntitlementRow {
	id: string;
	userId: string;
	entitlement: string;
	productId: string;
	status: "active" | "expired";
	store: string;
	environment?: string | null;
	expiresAt?: string | null;
	updatedAt: string;
}

/** Function context shape the handlers use; structurally compatible with
 *  `@pylonsync/functions`' ctx. */
export interface HandlerCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null; isAdmin?: boolean };
	request?: {
		headers: Record<string, string | undefined>;
		rawBody: string;
	};
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	runMutation: <T = unknown>(name: string, args: Record<string, unknown>) => Promise<T>;
	error: (code: string, message: string) => Error;
}
