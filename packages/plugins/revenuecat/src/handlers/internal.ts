/**
 * Internal mutation the webhook and the sync action write through.
 * Internal functions bypass entity policies (clients can never insert an
 * entitlement) and are not callable over HTTP.
 */

import { mutation, v } from "@pylonsync/functions";

import { entitlementEntityName } from "../manifest";
import type { HandlerCtx, RevenueCatConfig } from "../types";

export const INTERNAL_UPSERT = "_pylonRcUpsertEntitlement";

export function resolveWebhookAuth(ctx: HandlerCtx, cfg: RevenueCatConfig): string | undefined {
	if (cfg.getWebhookAuth) return cfg.getWebhookAuth(ctx);
	return ctx.env.REVENUECAT_WEBHOOK_AUTH;
}

export function resolveApiKey(ctx: HandlerCtx, cfg: RevenueCatConfig): string | undefined {
	if (cfg.getApiKey) return cfg.getApiKey(ctx);
	return ctx.env.REVENUECAT_SECRET_KEY ?? ctx.env.REVENUECAT_PUBLIC_KEY;
}

export interface UpsertArgs {
	userId: string;
	entitlement: string;
	productId: string;
	status: "active" | "expired";
	store: string;
	environment?: string | null;
	expiresAt?: string | null;
}

/** Upsert one (user, entitlement) row. Returns whether the row's status changed. */
export function upsertEntitlementHandler(cfg: RevenueCatConfig) {
	const entityName = entitlementEntityName(cfg);
	return mutation({
		internal: true,
		args: {
			userId: v.string(),
			entitlement: v.string(),
			productId: v.string(),
			status: v.string(),
			store: v.string(),
			environment: v.optional(v.string()),
			expiresAt: v.optional(v.string()),
		},
		async handler(ctx, args: UpsertArgs) {
			const existing = (await ctx.db.query(entityName, {
				userId: args.userId,
				entitlement: args.entitlement,
			})) as Array<{ id: string; status: string }>;
			const patch = {
				productId: args.productId,
				status: args.status,
				store: args.store,
				environment: args.environment ?? null,
				expiresAt: args.expiresAt ?? null,
				updatedAt: new Date().toISOString(),
			};
			if (existing.length > 0) {
				const before = existing[0];
				await ctx.db.update(entityName, before.id, patch);
				return { changed: before.status !== args.status, created: false };
			}
			await ctx.db.insert(entityName, {
				userId: args.userId,
				entitlement: args.entitlement,
				...patch,
			});
			return { changed: args.status === "active", created: true };
		},
	});
}
