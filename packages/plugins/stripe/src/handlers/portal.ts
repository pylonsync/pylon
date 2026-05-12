import { action, v } from "@pylonsync/functions";

import { stripeRequest } from "../client";
import { assertSafeRedirectUrl } from "../safe-url";
import type { HandlerCtx, StripeConfig } from "../types";
import {
	authorizeReference,
	resolveCustomerForReference,
	resolveSecretKey,
} from "./internal";

/**
 * Factory: returns an action handler that opens a Stripe Billing
 * Portal session for the reference. Customer manages their card,
 * downloads invoices, cancels the subscription — all without the
 * app having to build a billing UI.
 *
 * Portal config (return URL allow-list, products, branding) is set
 * once in the Stripe dashboard; this handler just mints sessions.
 */
export function createBillingPortalSessionHandler(cfg: StripeConfig) {
	return action({
		args: {
			referenceId: v.optional(v.string()),
			returnUrl: v.string(),
		},
		async handler(
			ctx: HandlerCtx,
			args: { referenceId?: string; returnUrl: string },
		) {
			const referenceId = args.referenceId ?? defaultReferenceId(ctx, cfg);
			if (!referenceId) {
				throw ctx.error(
					"NO_REFERENCE",
					"referenceId required (no active tenant or user)",
				);
			}
			await authorizeReference(ctx, cfg, referenceId, "billingPortal");

			assertSafeRedirectUrl(args.returnUrl, ctx.env.PYLON_PUBLIC_URL, {
				extraOrigins: ctx.env.PYLON_CORS_ORIGIN,
			});

			const { customerId } = await resolveCustomerForReference(
				ctx,
				cfg,
				referenceId,
			);

			const secretKey = resolveSecretKey(ctx, cfg);
			const session = await stripeRequest<{ id: string; url: string }>(
				{ secretKey, apiVersion: cfg.apiVersion },
				"POST",
				"/billing_portal/sessions",
				{ customer: customerId, return_url: args.returnUrl },
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
