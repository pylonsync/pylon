import { action } from "@pylonsync/functions";

import { INTERNAL_UPSERT, resolveApiKey } from "./internal";
import type { HandlerCtx, RevenueCatConfig } from "../types";

interface SubscriberResponse {
	subscriber?: {
		entitlements?: Record<
			string,
			{ expires_date?: string | null; product_identifier?: string }
		>;
	};
}

/** Pure mapping of a subscriber payload to row writes, exported for tests. */
export function entitlementsFromSubscriber(
	body: SubscriberResponse,
	now: number,
): Array<{ entitlement: string; productId: string; status: "active" | "expired"; expiresAt: string | null }> {
	const out: Array<{
		entitlement: string;
		productId: string;
		status: "active" | "expired";
		expiresAt: string | null;
	}> = [];
	for (const [entitlement, info] of Object.entries(body.subscriber?.entitlements ?? {})) {
		const expiresAt = info.expires_date ?? null;
		const active = expiresAt == null || new Date(expiresAt).getTime() > now;
		out.push({
			entitlement,
			productId: String(info.product_identifier ?? ""),
			status: active ? "active" : "expired",
			expiresAt,
		});
	}
	return out;
}

/**
 * Re-read the caller's entitlements from RevenueCat and write the rows.
 *
 * The webhook is the steady-state path; this closes the two gaps in it:
 * the purchasing device wants to unlock the moment the sheet dismisses
 * (the webhook may take seconds), and in local development RevenueCat
 * cannot reach the machine at all.
 *
 * Server-verified: the server asks RevenueCat about `ctx.auth.userId`;
 * a client can trigger the sync but never assert an entitlement.
 */
export function syncEntitlementsHandler(cfg: RevenueCatConfig) {
	return action({
		args: {},
		async handler(ctx: HandlerCtx) {
			const userId = ctx.auth.userId;
			if (!userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
			const key = resolveApiKey(ctx, cfg);
			if (!key) throw ctx.error("RC_NOT_CONFIGURED", "no RevenueCat API key on the server");

			const res = await fetch(
				`https://api.revenuecat.com/v1/subscribers/${encodeURIComponent(userId)}`,
				{ headers: { Authorization: `Bearer ${key}`, Accept: "application/json" } },
			);
			if (!res.ok) {
				throw ctx.error("RC_LOOKUP_FAILED", `RevenueCat returned ${res.status}`);
			}
			const body = (await res.json()) as SubscriberResponse;
			const rows = entitlementsFromSubscriber(body, Date.now());
			for (const row of rows) {
				const result = await ctx.runMutation<{ changed: boolean }>(INTERNAL_UPSERT, {
					userId,
					entitlement: row.entitlement,
					productId: row.productId,
					status: row.status,
					store: "sdk_sync",
					expiresAt: row.expiresAt,
				});
				if (result.changed) {
					const info = { userId, entitlement: row.entitlement, productId: row.productId, store: "sdk_sync" };
					if (row.status === "active") await cfg.hooks?.onEntitlementActive?.(ctx, info);
					else await cfg.hooks?.onEntitlementExpired?.(ctx, info);
				}
			}
			return { applied: rows.length, active: rows.filter((r) => r.status === "active").map((r) => r.entitlement) };
		},
	});
}
