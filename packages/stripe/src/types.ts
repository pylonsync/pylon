/**
 * Public type surface for `@pylonsync/stripe`.
 *
 * Shapes mirror Stripe's REST payloads at the level we actually use —
 * we don't pull in the official `stripe` SDK's types because they
 * change cadence + breadth doesn't match what apps need. If you need
 * a field that's missing, add it here.
 */

export interface StripePlan {
	/**
	 * Stable internal name. Stored on Subscription.plan + used as the
	 * `lookup_key` we set on Stripe prices via setup-stripe scripts.
	 * Lowercase, dash-separated.
	 */
	name: string;
	/**
	 * Stripe Price id (price_...) for the monthly tier. Resolved
	 * eagerly when the plan list is built (env-var indirection
	 * stays at the call site, not threaded through the plugin).
	 */
	priceId: string;
	/** Optional annual variant; surfaced as `annual: true` on upgrade. */
	annualPriceId?: string;
	/**
	 * Free-trial config. When set, checkout sessions for this plan
	 * default to `subscription_data.trial_period_days`. The plugin
	 * enforces one-trial-per-customer at the Stripe API level — Stripe
	 * itself rejects a second trial on the same customer when
	 * `trial_from_plan: true` is used, but we additionally short-
	 * circuit on the cloud side based on prior subscriptions to avoid
	 * the round-trip rejection.
	 */
	freeTrial?: {
		days: number;
	};
	/**
	 * Per-plan entitlement limits. Stored on the Subscription row's
	 * `limits` JSON column so app code can do quota checks without
	 * re-reading the plan list. Values are app-defined (recordings,
	 * seats, requests/mo — whatever the app meters).
	 */
	limits?: Record<string, number | boolean | string>;
	/**
	 * Seat-based billing. When set, `quantity` flows through to
	 * Stripe Subscription + the Subscription row stores `seats`.
	 */
	seatPriceId?: string;
}

/**
 * Reference-type for who owns a subscription. Most apps want `org`
 * (multi-seat workspaces); single-user apps set this to `user`. The
 * plugin doesn't care which — `referenceId` is just an opaque string
 * that gets stamped on Subscription.referenceId and looked up via
 * `getReference()` on webhook delivery.
 */
export type ReferenceType = "user" | "org" | "custom";

export interface StripeConfig {
	/**
	 * Resolved at runtime from `ctx.env.STRIPE_SECRET_KEY` unless an
	 * override is provided. Apps using multiple Stripe accounts (dev/
	 * prod under one binary) can wire a selector here.
	 */
	getSecretKey?: (ctx: HandlerCtx) => string;
	/**
	 * Resolved at runtime from `ctx.env.STRIPE_WEBHOOK_SECRET`.
	 * Same override pattern as `getSecretKey`.
	 */
	getWebhookSecret?: (ctx: HandlerCtx) => string;
	/**
	 * `user` — Subscription.referenceId = User.id. Customer is auto-
	 * created on first checkout, stored as `User.stripeCustomerId`.
	 * `org`  — Subscription.referenceId = Org.id. Customer is on
	 *          `Org.stripeCustomerId`. RBAC: only owners/admins can
	 *          upgrade/cancel.
	 * `custom` — caller supplies their own referenceId + customer
	 *           resolution via `resolveCustomer` hook.
	 */
	referenceType: ReferenceType;
	/**
	 * The plan catalog. Order matters: the first plan whose
	 * `priceId`/`lookupKey` matches a Stripe subscription's items
	 * wins on plan-derivation from webhook payloads.
	 */
	plans: StripePlan[];
	/**
	 * Which Stripe API version the client pins. Match this against
	 * what your dashboard webhook subscription is set to receive,
	 * otherwise field shapes drift.
	 */
	apiVersion?: string;
	/**
	 * Override the entity names the plugin reads/writes. Defaults
	 * mirror the manifest entities exported from this package.
	 */
	entities?: {
		subscription?: string;
		/** Where stripeCustomerId lives. Required. */
		customerHolder?: string;
	};
	/**
	 * Lifecycle hooks. Called server-side after the plugin has
	 * persisted its own state — apps add analytics, welcome emails,
	 * cache invalidation here.
	 */
	hooks?: StripeHooks;
	/**
	 * RBAC gate for subscription mutation actions. Returns `true` to
	 * allow, throws / returns `false` to deny. Defaults: for `org`
	 * referenceType, allows owners + admins; for `user`, allows the
	 * caller iff their auth.userId === referenceId.
	 */
	authorizeReference?: (
		ctx: HandlerCtx,
		args: { referenceId: string; action: SubscriptionAction },
	) => Promise<boolean> | boolean;
	/**
	 * `custom` referenceType requires this hook to map referenceId
	 * to a Stripe customer id (and create one if needed).
	 */
	resolveCustomer?: (
		ctx: HandlerCtx,
		referenceId: string,
	) => Promise<{ customerId: string; email?: string }>;
}

export type SubscriptionAction =
	| "upgrade"
	| "cancel"
	| "restore"
	| "billingPortal"
	| "list";

export interface StripeHooks {
	/** Fires after a Stripe customer is created. */
	onCustomerCreate?: (
		ctx: HandlerCtx,
		args: { referenceId: string; customerId: string; email?: string },
	) => Promise<void> | void;
	/**
	 * Fires after `customer.subscription.created` is persisted.
	 * Includes the resolved plan name + the StripeSubscription row.
	 */
	onSubscriptionActivate?: (
		ctx: HandlerCtx,
		args: { referenceId: string; plan: string; subscription: StripeSubscription },
	) => Promise<void> | void;
	/** Fires after `customer.subscription.updated` (plan changes, renewals). */
	onSubscriptionUpdate?: (
		ctx: HandlerCtx,
		args: { referenceId: string; plan: string; subscription: StripeSubscription },
	) => Promise<void> | void;
	/** Fires after `customer.subscription.deleted`. */
	onSubscriptionCancel?: (
		ctx: HandlerCtx,
		args: { referenceId: string; subscription: StripeSubscription },
	) => Promise<void> | void;
	/** Fires after every invoice event the plugin processes. */
	onInvoice?: (
		ctx: HandlerCtx,
		args: { referenceId: string; eventType: string; invoice: StripeInvoice },
	) => Promise<void> | void;
	/**
	 * Catch-all for any Stripe webhook event the plugin doesn't have
	 * built-in handling for. Useful for `payment_method.attached`,
	 * `charge.dispute.created`, etc.
	 */
	onEvent?: (ctx: HandlerCtx, event: StripeEvent) => Promise<void> | void;
	/**
	 * Inject extra params into the Stripe Checkout Session create
	 * call (tax, promo codes, idempotency key, custom_fields, etc.).
	 * Plugin merges over the base shape, so you can override defaults
	 * if you really want to.
	 */
	getCheckoutSessionParams?: (
		ctx: HandlerCtx,
		args: {
			referenceId: string;
			plan: StripePlan;
			customerId: string;
			successUrl: string;
			cancelUrl: string;
		},
	) => Promise<Record<string, unknown>> | Record<string, unknown>;
}

export interface StripeEvent {
	id: string;
	type: string;
	data: { object: Record<string, unknown> };
	created: number;
}

export interface StripeSubscription {
	id: string;
	status:
		| "active"
		| "past_due"
		| "canceled"
		| "incomplete"
		| "incomplete_expired"
		| "trialing"
		| "unpaid"
		| "paused";
	current_period_end?: number | null;
	cancel_at_period_end?: boolean;
	cancel_at?: number | null;
	canceled_at?: number | null;
	customer: string;
	trial_start?: number | null;
	trial_end?: number | null;
	items: {
		data: Array<{
			id?: string;
			current_period_end?: number | null;
			quantity?: number;
			price: {
				id: string;
				nickname?: string | null;
				lookup_key?: string | null;
			};
		}>;
	};
}

export interface StripeInvoice {
	id: string;
	status: string;
	amount_paid?: number;
	amount_due: number;
	period_start: number;
	period_end: number;
	customer: string;
	subscription?: string;
	hosted_invoice_url?: string;
}

/**
 * Handler-side `ctx` shape, narrowed to just the bits the plugin
 * touches. The real ctx from `@pylonsync/functions` is wider; we
 * type-guard the subset we depend on so the package compiles
 * without a tight dep on the functions runtime types.
 */
export interface HandlerCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null; isAdmin?: boolean };
	request?: {
		headers: Record<string, string | undefined>;
		rawBody: string;
	};
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	runMutation: <T = unknown>(
		name: string,
		args: Record<string, unknown>,
	) => Promise<T>;
	error: (code: string, message: string) => Error;
}
