import { action, v } from "@pylonsync/functions";

import { stripeRequest } from "../client";
import type { HandlerCtx, StripeConfig } from "../types";
import { authorizeReference, resolveSecretKey } from "./internal";

/**
 * Factory: returns an action that cancels the active subscription
 * for a reference. Two modes:
 *
 *   - `scheduleAtPeriodEnd: true` (default) — sets
 *     `cancel_at_period_end` so the customer keeps access through
 *     the paid window. The webhook handler updates the local row
 *     to `cancelAtPeriodEnd = true`. `restoreSubscription` can undo
 *     this any time before the period ends.
 *
 *   - `scheduleAtPeriodEnd: false` — immediate `DELETE`. Refunds
 *     are NOT issued automatically; that's a Stripe dashboard
 *     toggle. The webhook flips the row's status to `canceled`.
 */
export function cancelSubscriptionHandler(cfg: StripeConfig) {
	return action({
		args: {
			referenceId: v.optional(v.string()),
			scheduleAtPeriodEnd: v.optional(v.boolean()),
		},
		async handler(
			ctx: HandlerCtx,
			args: { referenceId?: string; scheduleAtPeriodEnd?: boolean },
		) {
			const referenceId = args.referenceId ?? defaultReferenceId(ctx, cfg);
			if (!referenceId) {
				throw ctx.error(
					"NO_REFERENCE",
					"referenceId required (no active tenant or user)",
				);
			}
			await authorizeReference(ctx, cfg, referenceId, "cancel");

			const sub = await ctx.runQuery<{
				stripeSubscriptionId: string;
			} | null>("_pylonStripeFindActiveSubForReference", { referenceId });
			if (!sub) {
				throw ctx.error("NOT_FOUND", "no active subscription for reference");
			}

			const scheduleAtPeriodEnd = args.scheduleAtPeriodEnd ?? true;
			const secretKey = resolveSecretKey(ctx, cfg);
			if (scheduleAtPeriodEnd) {
				await stripeRequest(
					{ secretKey, apiVersion: cfg.apiVersion },
					"POST",
					`/subscriptions/${sub.stripeSubscriptionId}`,
					{ cancel_at_period_end: true },
				);
			} else {
				await stripeRequest(
					{ secretKey, apiVersion: cfg.apiVersion },
					"DELETE",
					`/subscriptions/${sub.stripeSubscriptionId}`,
				);
			}
			// Local row update lands via the webhook; don't double-write
			// here because Stripe's response carries the canonical
			// status + period_end (and webhooks retry, so we want one
			// path that's idempotent).
			return { scheduled: scheduleAtPeriodEnd };
		},
	});
}

function defaultReferenceId(
	ctx: HandlerCtx,
	cfg: StripeConfig,
): string | null {
	if (cfg.referenceType === "user") return ctx.auth.userId ?? null;
	if (cfg.referenceType === "org") return ctx.auth.tenantId ?? null;
	return null;
}
