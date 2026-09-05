import { action } from "@pylonsync/functions";

import { INTERNAL_UPSERT, resolveWebhookAuth } from "./internal";
import type { HandlerCtx, RevenueCatConfig, RevenueCatEvent } from "../types";

/** Event types after which the named entitlements are no longer active. */
const ENDS_ACCESS = new Set(["EXPIRATION", "REFUND"]);

/** Event types that carry no entitlement change. */
const INFORMATIONAL = new Set(["TEST", "TRANSFER", "SUBSCRIBER_ALIAS"]);

export interface WebhookOutcome {
	ok: true;
	type: string;
	applied: number;
	skipped?: string;
}

/**
 * Pure event → row-status mapping, exported for tests.
 *
 * - INITIAL_PURCHASE / RENEWAL / NON_RENEWING_PURCHASE / UNCANCELLATION /
 *   PRODUCT_CHANGE / BILLING_ISSUE (still in grace) → active
 * - CANCELLATION → still active: auto-renew is off, access lasts until
 *   the EXPIRATION event
 * - EXPIRATION / REFUND → expired
 */
export function statusForEvent(type: string): "active" | "expired" | null {
	if (INFORMATIONAL.has(type)) return null;
	return ENDS_ACCESS.has(type) ? "expired" : "active";
}

export function entitlementIdsOf(event: RevenueCatEvent): string[] {
	if (Array.isArray(event.entitlement_ids) && event.entitlement_ids.length > 0) {
		return event.entitlement_ids.map(String);
	}
	if (event.entitlement_id) return [String(event.entitlement_id)];
	return [];
}

/** Check the shared secret. Accepts the raw value or `Bearer <value>`. */
export function webhookAuthorized(
	headerValue: string | undefined,
	expected: string | undefined,
): boolean {
	if (!expected) return false;
	const got = headerValue ?? "";
	return got === expected || got === `Bearer ${expected}`;
}

/**
 * RevenueCat webhook receiver. Every App Store / Play Billing / Stripe
 * event RevenueCat sees lands here and becomes entitlement rows.
 *
 * `app_user_id` must be the Pylon user id: the app calls
 * `Purchases.logIn(userId)` before any purchase. Anonymous ids
 * (`$RCAnonymousID:…`) are acknowledged and ignored.
 */
export function revenuecatWebhookHandler(cfg: RevenueCatConfig) {
	return action({
		auth: "public",
		args: {},
		async handler(ctx: HandlerCtx): Promise<WebhookOutcome> {
			if (!ctx.request) {
				throw ctx.error("BAD_INVOCATION", "revenuecatWebhook requires HTTP request context");
			}
			const expected = resolveWebhookAuth(ctx, cfg);
			if (!expected) {
				throw ctx.error("RC_NOT_CONFIGURED", "REVENUECAT_WEBHOOK_AUTH is not set");
			}
			if (!webhookAuthorized(ctx.request.headers["authorization"], expected)) {
				throw ctx.error("FORBIDDEN", "bad webhook authorization");
			}

			let event: RevenueCatEvent;
			try {
				const body = JSON.parse(ctx.request.rawBody) as { event?: RevenueCatEvent };
				event = body.event ?? ({ type: "" } as RevenueCatEvent);
			} catch {
				throw ctx.error("BAD_BODY", "invalid JSON");
			}
			const type = String(event.type ?? "");
			const userId = String(event.app_user_id ?? "");
			if (!userId || userId.startsWith("$RCAnonymous")) {
				return { ok: true, type, applied: 0, skipped: "no app user id" };
			}
			const status = statusForEvent(type);
			const entitlements = entitlementIdsOf(event);
			if (!status || entitlements.length === 0) {
				await cfg.hooks?.onEvent?.(ctx, event);
				return { ok: true, type, applied: 0, skipped: "no entitlement change" };
			}

			const productId = String(event.product_id ?? "");
			const store = String(event.store ?? "").toLowerCase();
			const expiresAt = event.expiration_at_ms
				? new Date(Number(event.expiration_at_ms)).toISOString()
				: null;
			for (const entitlement of entitlements) {
				const result = await ctx.runMutation<{ changed: boolean }>(INTERNAL_UPSERT, {
					userId,
					entitlement,
					productId,
					status,
					store,
					environment: event.environment ?? null,
					expiresAt,
				});
				if (result.changed) {
					const info = { userId, entitlement, productId, store };
					if (status === "active") await cfg.hooks?.onEntitlementActive?.(ctx, info);
					else await cfg.hooks?.onEntitlementExpired?.(ctx, info);
				}
			}
			await cfg.hooks?.onEvent?.(ctx, event);
			return { ok: true, type, applied: entitlements.length };
		},
	});
}
