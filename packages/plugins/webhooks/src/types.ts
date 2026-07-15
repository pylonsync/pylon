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
	/** Hex-encoded HMAC secret. The plugin manifest always marks this field
	 *  server-only and optionally encrypts it at rest. */
	secret: string;
	/** Subset of event types this endpoint subscribes to. Empty = all. */
	eventTypes: string[];
	/**
	 * Headers to attach on every delivery. Useful for routing /
	 * authentication beyond the HMAC signature. The plugin manifest always
	 * marks this field server-only and optionally encrypts it at rest.
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
	 * Encrypt endpoint secrets and custom headers at rest. Requires a 32-byte
	 * `PYLON_ENCRYPTION_KEY`; endpoint writes fail with
	 * `ENCRYPTION_NOT_CONFIGURED` when the key is absent. Defaults to `false`
	 * so enabling the plugin does not break existing deployments. Credentials
	 * remain server-only regardless of this setting.
	 */
	encryptCredentials?: boolean;
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
	auth: {
		userId?: string | null;
		tenantId?: string | null;
		/**
		 * Optional — only present on Pylon framework v0.3.118+. The
		 * dispatch path uses it (when available) to elevate the
		 * caller to admin before scheduling the internal:true
		 * `_pylonWebhookDeliver` worker. Without elevation the
		 * scheduler's public-to-internal gate refuses the enqueue
		 * and no webhooks deliver — see dispatch.ts comment.
		 */
		elevate?: (options: { admin: boolean; reason: string }) => Promise<void>;
	};
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
