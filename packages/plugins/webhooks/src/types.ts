/**
 * `@pylonsync/webhooks` — outbound webhook delivery for Pylon apps.
 *
 * Svix-style HMAC-SHA256 signed payloads with timestamp +
 * replay-window protection, exponential-backoff retry schedule,
 * secret rotation overlap, per-endpoint event filtering,
 * idempotent delivery via stable event ids.
 *
 * Mental model:
 *   - Apps register **endpoints** at runtime: customer-supplied
 *     URLs that should receive certain event types.
 *   - The app's domain code calls `dispatch(ctx, { type, payload })`
 *     when something happens.
 *   - The plugin enqueues delivery to each matching endpoint via
 *     Pylon's job queue.
 *   - Worker pulls the job, signs the payload with the endpoint's
 *     secret, POSTs it. On failure, re-enqueues with the next
 *     retry interval (5s → 5m → 30m → 2h → 5h → 10h → 14h → dead).
 */

export interface WebhookEvent<T = unknown> {
	/** Stable id. Receivers dedup on this. */
	id: string;
	type: string;
	occurredAt: string;
	data: T;
}

export interface WebhookEndpoint {
	id: string;
	/** Logical bucket — usually the customer/org id. */
	applicationId: string;
	url: string;
	/** Hex-encoded HMAC secret. Plaintext on the row by design — only
	 *  the customer's app reads + uses it, and Pylon's at-rest crypto
	 *  is the secret-encryption layer apps wire when they want it. */
	secret: string;
	/** Subset of event types this endpoint subscribes to. Empty = all. */
	eventTypes: string[];
	/**
	 * Headers to attach on every delivery. Useful for routing /
	 * authentication beyond the HMAC signature.
	 */
	headers?: Record<string, string>;
	/** Disabled endpoints don't receive deliveries. */
	disabled?: boolean;
	createdAt: string;
}

export interface WebhookConfig {
	entities?: {
		endpoint?: string;
		attempt?: string;
	};
	/**
	 * Retry schedule (seconds offset from initial attempt). Default
	 * matches Svix's published schedule. After exhausting the
	 * schedule, the attempt is marked `dead` and the destination
	 * goes to the dead-letter queue.
	 */
	retrySchedule?: number[];
	/** Replay window for signature validation. Default 300s. */
	replayToleranceSecs?: number;
	/**
	 * App identifier resolver. For multi-tenant apps, this returns
	 * the application id (org id) from the dispatch context — used
	 * to filter endpoints to the right customer.
	 */
	getApplicationId?: (
		ctx: WebhookCtx,
		payload: WebhookEvent,
	) => string | null | Promise<string | null>;
}

export interface WebhookCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null };
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	runMutation: <T = unknown>(
		name: string,
		args: Record<string, unknown>,
	) => Promise<T>;
	scheduler: {
		runAfter: (
			ms: number,
			fn: string,
			args: Record<string, unknown>,
		) => Promise<string>;
	};
	error: (code: string, message: string) => Error;
}

export interface DispatchInput {
	type: string;
	data: unknown;
	/**
	 * Stable event id. Apps that produce events idempotently (e.g.
	 * "subscription.updated" from a Stripe webhook) should pass
	 * the upstream event id here so re-deliveries dedup. Auto-
	 * generated when absent.
	 */
	id?: string;
	/** Override the auto-resolved application id. */
	applicationId?: string;
	occurredAt?: string;
}

export interface DeliveryAttempt {
	id: string;
	eventId: string;
	endpointId: string;
	url: string;
	attempt: number;
	status: "pending" | "succeeded" | "failed" | "dead";
	httpStatus?: number;
	error?: string;
	scheduledAt: string;
	deliveredAt?: string;
}
