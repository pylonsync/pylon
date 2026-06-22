import { action } from "@pylonsync/functions";

import { periodEndFromSubscription, planFromSubscription } from "../plans";
import { verifyStripeSignature } from "../signature";
import type {
	HandlerCtx,
	StripeConfig,
	StripeEvent,
	StripeInvoice,
	StripeSubscription,
} from "../types";
import { resolveWebhookSecret } from "./internal";

/**
 * Factory: returns the unified webhook handler. Verifies signature,
 * routes by event type, upserts the canonical Subscription row,
 * fires lifecycle hooks.
 *
 * Event coverage (matches the better-auth Stripe plugin):
 *
 *   customer.subscription.created   → upsert row, status=active|trialing
 *   customer.subscription.updated   → upsert row, reflect new plan/seats/period
 *   customer.subscription.deleted   → row → status=canceled
 *   customer.subscription.trial_will_end → onEvent hook only
 *   invoice.created/finalized/paid/payment_failed/voided → onInvoice hook
 *   anything else                   → cfg.hooks.onEvent
 *
 * Mounted at `POST /api/webhooks/stripeWebhook` by Pylon's webhook
 * router. Configure your Stripe dashboard endpoint with that URL
 * and the signing secret as `STRIPE_WEBHOOK_SECRET`.
 */
export function stripeWebhookHandler(cfg: StripeConfig) {
	return action({
		// Webhook receivers are unauthenticated by nature: Stripe POSTs
		// anonymously, so the default `auth: "user"` 401s every delivery
		// before the handler runs. The real auth boundary is the signature
		// check below (verifyStripeSignature against STRIPE_WEBHOOK_SECRET),
		// not a user session.
		auth: "public",
		args: {},
		async handler(ctx: HandlerCtx) {
			if (!ctx.request) {
				throw ctx.error(
					"BAD_INVOCATION",
					"stripeWebhook requires HTTP request context",
				);
			}
			const secret = resolveWebhookSecret(ctx, cfg);
			const sigHeader = ctx.request.headers["stripe-signature"];
			const ok = await verifyStripeSignature(
				secret,
				ctx.request.rawBody,
				sigHeader,
			);
			if (ok !== true) {
				throw ctx.error("INVALID_SIGNATURE", `stripe sig: ${ok}`);
			}

			let event: StripeEvent;
			try {
				event = JSON.parse(ctx.request.rawBody) as StripeEvent;
			} catch {
				throw ctx.error("BAD_BODY", "invalid JSON");
			}

			switch (event.type) {
				case "customer.subscription.created":
				case "customer.subscription.updated":
				case "customer.subscription.deleted": {
					const sub = event.data.object as unknown as StripeSubscription;
					await handleSubscriptionEvent(ctx, cfg, event, sub);
					return { ok: true, type: event.type };
				}
				case "invoice.created":
				case "invoice.finalized":
				case "invoice.paid":
				case "invoice.payment_failed":
				case "invoice.voided": {
					const inv = event.data.object as unknown as StripeInvoice;
					await handleInvoiceEvent(ctx, cfg, event, inv);
					return { ok: true, type: event.type };
				}
				default:
					if (cfg.hooks?.onEvent) await cfg.hooks.onEvent(ctx, event);
					return { ignored: event.type };
			}
		},
	});
}

async function handleSubscriptionEvent(
	ctx: HandlerCtx,
	cfg: StripeConfig,
	event: StripeEvent,
	sub: StripeSubscription,
): Promise<void> {
	const referenceId = await referenceFromCustomerId(ctx, cfg, sub.customer);
	if (!referenceId) {
		// No mapping — log + drop. The app likely deleted the
		// reference; Stripe will retry the webhook for a while and
		// eventually mark it as failed.
		return;
	}
	const { plan } = planFromSubscription(cfg.plans, sub);
	const planName = plan?.name ?? "pending";
	const now = new Date().toISOString();
	await ctx.runMutation("_pylonStripeUpsertSubscription", {
		referenceId,
		stripeCustomerId: sub.customer,
		stripeSubscriptionId: sub.id,
		plan: planName,
		status: sub.status,
		seats: sub.items.data[0]?.quantity ?? 1,
		currentPeriodEnd: periodEndFromSubscription(sub),
		cancelAtPeriodEnd: !!sub.cancel_at_period_end,
		canceledAt: sub.canceled_at
			? new Date(sub.canceled_at * 1000).toISOString()
			: null,
		trialEnd: sub.trial_end ? new Date(sub.trial_end * 1000).toISOString() : null,
		limits: plan?.limits ? JSON.stringify(plan.limits) : null,
		createdAt: now,
		updatedAt: now,
	});
	if (event.type === "customer.subscription.created" && cfg.hooks?.onSubscriptionActivate) {
		await cfg.hooks.onSubscriptionActivate(ctx, { referenceId, plan: planName, subscription: sub });
	} else if (event.type === "customer.subscription.updated" && cfg.hooks?.onSubscriptionUpdate) {
		await cfg.hooks.onSubscriptionUpdate(ctx, { referenceId, plan: planName, subscription: sub });
	} else if (event.type === "customer.subscription.deleted" && cfg.hooks?.onSubscriptionCancel) {
		await cfg.hooks.onSubscriptionCancel(ctx, { referenceId, subscription: sub });
	}
}

async function handleInvoiceEvent(
	ctx: HandlerCtx,
	cfg: StripeConfig,
	event: StripeEvent,
	inv: StripeInvoice,
): Promise<void> {
	const referenceId = await referenceFromCustomerId(ctx, cfg, inv.customer);
	if (!referenceId) return;
	if (cfg.hooks?.onInvoice) {
		await cfg.hooks.onInvoice(ctx, {
			referenceId,
			eventType: event.type,
			invoice: inv,
		});
	}
}

async function referenceFromCustomerId(
	ctx: HandlerCtx,
	cfg: StripeConfig,
	customerId: string,
): Promise<string | null> {
	const holder = cfg.entities?.customerHolder ??
		(cfg.referenceType === "user" ? "User" : "Org");
	const row = await ctx.runQuery<{ id: string } | null>(
		"_pylonStripeFindByCustomerId",
		{ entity: holder, stripeCustomerId: customerId },
	);
	return row?.id ?? null;
}
