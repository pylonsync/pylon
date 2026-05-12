import { action, v } from "@pylonsync/functions";

import { stripeRequest } from "../client";
import { findPlanByName } from "../plans";
import { assertSafeRedirectUrl } from "../safe-url";
import type { HandlerCtx, StripeConfig } from "../types";
import {
	authorizeReference,
	resolveCustomerForReference,
	subscriptionEntity,
} from "./internal";

/**
 * Factory: returns an action handler for creating a Checkout Session.
 *
 * Flow:
 *   1. authorize the referenceId for `upgrade`
 *   2. resolve the Stripe customer (create on first call, persist on
 *      the customer-holder entity)
 *   3. validate the success/cancel URLs against PYLON_PUBLIC_URL +
 *      app-provided extras (closes the "yapless.com vs getyapless.com"
 *      bug class)
 *   4. apply free-trial config from the plan catalog
 *   5. POST /v1/checkout/sessions with merged hook params
 *   6. return the URL — caller redirects the user
 */
export function createCheckoutSessionHandler(cfg: StripeConfig) {
	return action({
		args: {
			plan: v.string(),
			referenceId: v.optional(v.string()),
			successUrl: v.string(),
			cancelUrl: v.string(),
			annual: v.optional(v.boolean()),
			seats: v.optional(v.number()),
		},
		async handler(
			ctx: HandlerCtx,
			args: {
				plan: string;
				referenceId?: string;
				successUrl: string;
				cancelUrl: string;
				annual?: boolean;
				seats?: number;
			},
		) {
			const referenceId = args.referenceId ?? defaultReferenceId(ctx, cfg);
			if (!referenceId) {
				throw ctx.error(
					"NO_REFERENCE",
					"referenceId required (no active tenant or user)",
				);
			}
			await authorizeReference(ctx, cfg, referenceId, "upgrade");

			const plan = findPlanByName(cfg.plans, args.plan);
			if (!plan) {
				throw ctx.error("UNKNOWN_PLAN", `plan ${args.plan} not in catalog`);
			}
			const priceId = args.annual ? plan.annualPriceId : plan.priceId;
			if (!priceId) {
				throw ctx.error(
					"NO_PRICE",
					`plan ${args.plan} has no ${args.annual ? "annual" : "monthly"} priceId`,
				);
			}

			assertSafeRedirectUrl(args.successUrl, ctx.env.PYLON_PUBLIC_URL, {
				extraOrigins: ctx.env.PYLON_CORS_ORIGIN,
			});
			assertSafeRedirectUrl(args.cancelUrl, ctx.env.PYLON_PUBLIC_URL, {
				extraOrigins: ctx.env.PYLON_CORS_ORIGIN,
			});

			const customer = await resolveCustomerForReference(ctx, cfg, referenceId);

			const secretKey = resolveSecretKey(ctx, cfg);

			// Block double-trials: if the customer already has any
			// Subscription row, we set `trial_period_days` to 0
			// regardless of plan config. Stripe also enforces this
			// server-side via the customer's trial history, but
			// short-circuiting here avoids the round-trip rejection
			// AND prevents giving customers a UX "you just got a 14-day
			// trial" message that Stripe later silently strips.
			const priorSubs = await ctx.runQuery<Array<{ id: string }>>(
				"_pylonStripeListSubsForReference",
				{ referenceId },
			);
			const trialDays =
				priorSubs.length === 0 && plan.freeTrial?.days
					? plan.freeTrial.days
					: 0;

			const baseParams: Record<string, unknown> = {
				mode: "subscription",
				customer: customer.customerId,
				success_url: args.successUrl,
				cancel_url: args.cancelUrl,
				line_items: [
					{
						price: priceId,
						quantity: args.seats ?? 1,
					},
				],
				allow_promotion_codes: true,
				client_reference_id: referenceId,
				metadata: { referenceId, plan: plan.name },
				subscription_data: {
					metadata: { referenceId, plan: plan.name },
					...(trialDays > 0 ? { trial_period_days: trialDays } : {}),
				},
			};

			const extra = cfg.hooks?.getCheckoutSessionParams
				? await cfg.hooks.getCheckoutSessionParams(ctx, {
						referenceId,
						plan,
						customerId: customer.customerId,
						successUrl: args.successUrl,
						cancelUrl: args.cancelUrl,
					})
				: {};
			const merged = { ...baseParams, ...extra };

			const session = await stripeRequest<{ id: string; url: string }>(
				{ secretKey, apiVersion: cfg.apiVersion },
				"POST",
				"/checkout/sessions",
				merged,
			);
			return { url: session.url, id: session.id };
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

function resolveSecretKey(ctx: HandlerCtx, cfg: StripeConfig): string {
	if (cfg.getSecretKey) return cfg.getSecretKey(ctx);
	const v = ctx.env.STRIPE_SECRET_KEY;
	if (!v) {
		throw ctx.error("MISSING_ENV", "STRIPE_SECRET_KEY not set");
	}
	return v;
}
