/**
 * `@pylonsync/stripe/entitlement` — the part of the plugin that runs in app
 * code (browser, React Native).
 *
 * No server imports, so Metro and browser bundlers can take it. The package
 * root and `./client` are server-only: `client.ts` is the REST client and
 * carries the secret key.
 *
 * This exists because an app that sells on the web AND through the stores
 * has two sources of truth — a StripeSubscription row for a web purchase and
 * an RcEntitlement row for a store purchase — and a subscriber has exactly
 * one of them. Reading only the store's rows is how someone pays on the web
 * and gets nothing on their phone.
 */

/** One row of the subscription entity, as the client reads it through sync. */
export interface StripeSubscriptionRow {
	id: string;
	referenceId: string;
	plan: string;
	/** Stripe's own status: active, trialing, past_due, canceled, … */
	status: string;
	currentPeriodEnd?: string | null;
	cancelAtPeriodEnd?: boolean | null;
}

/**
 * Statuses that still carry access.
 *
 * `past_due` does: every card retry takes days, and cutting a paying
 * customer off over a payment that will clear tomorrow costs more than the
 * few days it saves. `trialing` does, because a trial is access. Nothing
 * else — in particular `incomplete`, where the first payment never cleared,
 * which is how someone gets a subscription by abandoning a card form.
 */
const LIVE_STATUSES = new Set(["active", "trialing", "past_due"]);

/**
 * Is there a live Stripe subscription in a list of rows read through sync?
 *
 * A row whose paid period has elapsed is treated as inactive even if the
 * webhook carrying that news has not arrived yet.
 */
export function hasStripeSubscription(
	rows: ReadonlyArray<{ status: string; currentPeriodEnd?: string | null }>,
	now: number = Date.now(),
): boolean {
	return rows.some((r) => {
		if (!LIVE_STATUSES.has(r.status)) return false;
		if (r.currentPeriodEnd == null) return false;
		const end = new Date(r.currentPeriodEnd).getTime();
		return Number.isFinite(end) && end > now;
	});
}
