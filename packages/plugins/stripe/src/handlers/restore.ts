import { action, v } from "@pylonsync/functions";

import { stripeRequest } from "../client";
import type { HandlerCtx, StripeConfig } from "../types";
import { authorizeReference, resolveSecretKey } from "./internal";

/**
 * Factory: returns an action that undoes a pending cancellation
 * (i.e. a subscription with `cancel_at_period_end: true` that
 * hasn't yet reached its period end). Only works while
 * `subscription.status` is still `active` and the period hasn't
 * expired — once Stripe has actually canceled the sub, restoring
 * requires a new checkout.
 */
export function restoreSubscriptionHandler(cfg: StripeConfig) {
	return action({
		args: {
			referenceId: v.optional(v.string()),
		},
		async handler(ctx: HandlerCtx, args: { referenceId?: string }) {
			const referenceId = args.referenceId ?? defaultReferenceId(ctx, cfg);
			if (!referenceId) {
				throw ctx.error(
					"NO_REFERENCE",
					"referenceId required (no active tenant or user)",
				);
			}
			await authorizeReference(ctx, cfg, referenceId, "restore");

			const sub = await ctx.runQuery<{
				stripeSubscriptionId: string;
				cancelAtPeriodEnd?: boolean;
				status: string;
			} | null>("_pylonStripeFindActiveSubForReference", { referenceId });
			if (!sub) {
				throw ctx.error("NOT_FOUND", "no subscription for reference");
			}
			if (!sub.cancelAtPeriodEnd) {
				throw ctx.error(
					"NOT_CANCELING",
					"subscription is not scheduled to cancel",
				);
			}

			const secretKey = resolveSecretKey(ctx, cfg);
			await stripeRequest(
				{ secretKey, apiVersion: cfg.apiVersion },
				"POST",
				`/subscriptions/${sub.stripeSubscriptionId}`,
				{ cancel_at_period_end: false },
			);
			return { restored: true };
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
