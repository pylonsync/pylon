/**
 * Plan resolution helpers — map a Stripe subscription item back to
 * the app's plan name, and the reverse (plan name → price id) for
 * checkout session creation.
 *
 * Stripe sometimes elides `nickname` / `lookup_key` on webhook
 * payloads (the price object is shallow-expanded), which made
 * yapless's old plan-derivation function silently demote orgs to
 * "pending" after a successful checkout. We try three signals in
 * order so the resolver still works when any one is missing:
 *
 *   1. price.lookup_key (operator-set, most stable)
 *   2. price.nickname   (often elided on webhooks)
 *   3. price.id matched against the plan catalog's priceId/annualPriceId
 */

import type { StripePlan, StripeSubscription } from "./types";

export function findPlanByName(
	plans: StripePlan[],
	name: string,
): StripePlan | undefined {
	return plans.find((p) => p.name === name);
}

export interface ResolvedPlan {
	plan: StripePlan | null;
	annual: boolean;
}

export function planFromSubscription(
	plans: StripePlan[],
	sub: StripeSubscription,
): ResolvedPlan {
	const item = sub.items.data[0];
	if (!item) return { plan: null, annual: false };
	const priceId = item.price.id;
	const key = item.price.lookup_key ?? item.price.nickname ?? undefined;
	if (key) {
		const byKey = plans.find((p) => p.name === key);
		if (byKey) return { plan: byKey, annual: byKey.annualPriceId === priceId };
	}
	for (const p of plans) {
		if (p.priceId === priceId) return { plan: p, annual: false };
		if (p.annualPriceId === priceId) return { plan: p, annual: true };
	}
	return { plan: null, annual: false };
}

export function periodEndFromSubscription(
	sub: StripeSubscription,
): string | null {
	const ts =
		sub.items.data[0]?.current_period_end ?? sub.current_period_end ?? null;
	if (!ts) return null;
	return new Date(ts * 1000).toISOString();
}
