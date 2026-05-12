/**
 * Shared internals used by every handler in the package.
 *
 * Why this file exists: the plugin needs three primitives that
 * cross-cut every action — authorize a reference, look up / create
 * a Stripe customer for a reference, and resolve which mutation
 * the app declared for persisting customer-id / subscription rows.
 * Keeping them here means the per-handler files (checkout, portal,
 * cancel, etc.) stay focused on the Stripe-side flow.
 */

import { stripeRequest } from "../client";
import type { HandlerCtx, StripeConfig, SubscriptionAction } from "../types";

export function subscriptionEntity(cfg: StripeConfig): string {
	return cfg.entities?.subscription ?? "StripeSubscription";
}

export function customerHolderEntity(cfg: StripeConfig): string {
	if (cfg.entities?.customerHolder) return cfg.entities.customerHolder;
	if (cfg.referenceType === "org") return "Org";
	if (cfg.referenceType === "user") return "User";
	throw new Error(
		`@pylonsync/stripe: customerHolder entity required for referenceType=custom`,
	);
}

/**
 * RBAC gate. Defaults:
 *   - `org`: owners + admins allowed (looked up via OrgMember)
 *   - `user`: only the caller themselves
 *   - `custom`: caller must supply `cfg.authorizeReference`
 * Always throws on deny — callers don't need to handle false return.
 */
export async function authorizeReference(
	ctx: HandlerCtx,
	cfg: StripeConfig,
	referenceId: string,
	action: SubscriptionAction,
): Promise<void> {
	if (!ctx.auth.userId) {
		throw ctx.error("UNAUTHENTICATED", "sign in to manage subscriptions");
	}
	if (cfg.authorizeReference) {
		const ok = await cfg.authorizeReference(ctx, { referenceId, action });
		if (!ok) throw ctx.error("FORBIDDEN", `not authorized to ${action}`);
		return;
	}
	if (ctx.auth.isAdmin) return;
	if (cfg.referenceType === "user") {
		if (ctx.auth.userId !== referenceId) {
			throw ctx.error("FORBIDDEN", "can only manage your own subscription");
		}
		return;
	}
	if (cfg.referenceType === "org") {
		const members = await ctx.runQuery<Array<{ role: string }>>(
			"_pylonStripeOrgMembership",
			{ orgId: referenceId, userId: ctx.auth.userId },
		);
		const role = members[0]?.role;
		if (role !== "owner" && role !== "admin") {
			throw ctx.error(
				"FORBIDDEN",
				`only org owners or admins can ${action}`,
			);
		}
		return;
	}
	throw ctx.error(
		"NO_AUTHORIZER",
		"referenceType=custom requires authorizeReference hook",
	);
}

/**
 * Find an existing Stripe customer for the reference, or create one
 * and persist its id on the customer-holder entity. Idempotent.
 */
export async function resolveCustomerForReference(
	ctx: HandlerCtx,
	cfg: StripeConfig,
	referenceId: string,
): Promise<{ customerId: string; email?: string }> {
	if (cfg.referenceType === "custom") {
		if (!cfg.resolveCustomer) {
			throw ctx.error(
				"NO_RESOLVER",
				"referenceType=custom requires resolveCustomer hook",
			);
		}
		return cfg.resolveCustomer(ctx, referenceId);
	}

	const holder = customerHolderEntity(cfg);
	const row = await ctx.runQuery<{
		id: string;
		stripeCustomerId?: string | null;
		email?: string;
		name?: string;
	} | null>("_pylonStripeGetCustomerHolder", { entity: holder, id: referenceId });
	if (!row) {
		throw ctx.error("NOT_FOUND", `${holder} ${referenceId} not found`);
	}
	if (row.stripeCustomerId) {
		return { customerId: row.stripeCustomerId, email: row.email };
	}

	const secretKey = resolveSecretKey(ctx, cfg);
	const created = await stripeRequest<{ id: string }>(
		{ secretKey, apiVersion: cfg.apiVersion },
		"POST",
		"/customers",
		{
			email: row.email,
			name: row.name,
			metadata: { referenceId, referenceType: cfg.referenceType },
		},
	);
	await ctx.runMutation("_pylonStripeSetCustomerId", {
		entity: holder,
		id: referenceId,
		stripeCustomerId: created.id,
	});
	if (cfg.hooks?.onCustomerCreate) {
		await cfg.hooks.onCustomerCreate(ctx, {
			referenceId,
			customerId: created.id,
			email: row.email,
		});
	}
	return { customerId: created.id, email: row.email };
}

export function resolveSecretKey(ctx: HandlerCtx, cfg: StripeConfig): string {
	if (cfg.getSecretKey) return cfg.getSecretKey(ctx);
	const v = ctx.env.STRIPE_SECRET_KEY;
	if (!v) {
		throw ctx.error("MISSING_ENV", "STRIPE_SECRET_KEY not set");
	}
	return v;
}

export function resolveWebhookSecret(
	ctx: HandlerCtx,
	cfg: StripeConfig,
): string {
	if (cfg.getWebhookSecret) return cfg.getWebhookSecret(ctx);
	const v = ctx.env.STRIPE_WEBHOOK_SECRET;
	if (!v) {
		throw ctx.error("MISSING_ENV", "STRIPE_WEBHOOK_SECRET not set");
	}
	return v;
}
