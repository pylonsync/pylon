/**
 * `@pylonsync/revenuecat/client` — the part of the plugin that runs in
 * app code (browser, React Native). It has no dependency on
 * `@pylonsync/functions`, so Metro and browser bundlers can import it.
 * The package root is server-only (handlers, manifest fragment).
 */

/** One row of the entitlement entity, as the client reads it through sync. */
export interface RcEntitlementRow {
	id: string;
	userId: string;
	entitlement: string;
	productId: string;
	status: "active" | "expired";
	store: string;
	environment?: string | null;
	expiresAt?: string | null;
	updatedAt: string;
}

/**
 * Is `entitlement` active in a list of rows the app read through sync?
 * Treats a row with a past `expiresAt` as inactive even if the webhook
 * has not delivered the expiration yet.
 */
export function hasEntitlement(
	rows: ReadonlyArray<{ entitlement: string; status: string; expiresAt?: string | null }>,
	entitlement: string,
	now: number = Date.now(),
): boolean {
	return rows.some(
		(r) =>
			r.entitlement === entitlement &&
			r.status === "active" &&
			(r.expiresAt == null || new Date(r.expiresAt).getTime() > now),
	);
}
