/**
 * `@pylonsync/webhooks` — outbound webhook delivery.
 *
 * Three primitives:
 *   - `signWebhook`/`verifyWebhook` — Svix-compatible HMAC-SHA256
 *     signature with timestamp + replay window. Receivers using
 *     Svix's reference verifier (Clerk, Resend, anyone with a
 *     `whsec_*` secret) work unchanged.
 *   - `dispatch(ctx, cfg, { type, data })` — fan out an event to
 *     every endpoint subscribed to `type`. Enqueues delivery jobs
 *     via `ctx.scheduler.runAfter`.
 *   - `deliverNow(ctx, cfg, attemptId)` — the worker side. Signs
 *     and POSTs, re-enqueues on failure with the next retry interval.
 *
 * Manifest fragment adds two entities:
 *   - `WebhookEndpoint` — customer-registered URLs
 *   - `WebhookAttempt`  — delivery audit log (one row per attempt)
 *
 * Schema is intentionally minimal: apps that want richer
 * application/event-type catalogs (Svix's full model) layer that on
 * top, the plugin doesn't enforce a structure.
 */

export { signWebhook, verifyWebhook } from "./signature";
export type {
	SignOptions,
	WebhookSignaturePayload,
	SignatureError,
} from "./signature";
export type {
	WebhookEvent,
	WebhookEndpoint,
	WebhookConfig,
	WebhookCtx,
	DispatchInput,
	DeliveryAttempt,
} from "./types";

import { buildWebhookManifest, type WebhookManifestFragment } from "./manifest";
import { dispatch } from "./dispatch";
import { internalHandlers } from "./handlers";
import type { WebhookConfig } from "./types";

export { buildWebhookManifest } from "./manifest";
export type { WebhookManifestFragment } from "./manifest";
export { dispatch } from "./dispatch";

export const DEFAULT_RETRY_SCHEDULE_SECS = [
	5, // 5s
	300, // 5m
	1800, // 30m
	7200, // 2h
	18000, // 5h
	36000, // 10h
	50400, // 14h
];

export interface WebhooksPlugin {
	config: WebhookConfig;
	manifest: WebhookManifestFragment;
	handlers: Record<string, unknown>;
	dispatch: typeof dispatch;
}

export function webhooks(cfg: WebhookConfig = {}): WebhooksPlugin {
	const resolved: WebhookConfig = {
		retrySchedule: cfg.retrySchedule ?? DEFAULT_RETRY_SCHEDULE_SECS,
		replayToleranceSecs: cfg.replayToleranceSecs ?? 300,
		...cfg,
	};
	return {
		config: resolved,
		manifest: buildWebhookManifest(resolved),
		handlers: internalHandlers(resolved),
		dispatch,
	};
}
