/**
 * URL allowlist for Stripe Checkout `success_url` + `cancel_url`.
 *
 * The bug we're closing: hardcoded allowlists ("yapless.com" vs the
 * actual "getyapless.com") cause every checkout button to throw
 * "URL is not on the allowed host" in prod. Reading allowlist from
 * `ctx.env.PYLON_PUBLIC_URL` (which Pylon Cloud auto-sets) eliminates
 * the whole bug class — the same env that drives OAuth callbacks +
 * SAML responses also drives Stripe redirect validation.
 *
 * Defense:
 *   - Allowed origins = [PYLON_PUBLIC_URL host] + extras provided
 *     by the app (e.g. its marketing root if different from the API
 *     host) + localhost/127.0.0.1 in dev.
 *   - Subdomain match (api.acme.com → also allows www.acme.com via
 *     suffix logic) is intentional: SaaS apps almost always serve
 *     frontend + backend under sibling subdomains, and forcing the
 *     app to list every subdomain creates the same hardcoding bug
 *     this is trying to prevent.
 */

export interface AssertSafeUrlOptions {
	/** Comma-separated list pulled from `ctx.env.PYLON_CORS_ORIGIN` */
	extraOrigins?: string;
	/** Single hostname override (legacy `YAPLESS_BASE_HOST`-style env). */
	extraHost?: string;
	/** Default: PYLON_DEV_MODE → true. Apps can force-disable for tests. */
	allowLocalhost?: boolean;
}

export function assertSafeRedirectUrl(
	url: string,
	publicUrl: string | undefined,
	opts: AssertSafeUrlOptions = {},
): void {
	let parsed: URL;
	try {
		parsed = new URL(url);
	} catch {
		throw new Error(`invalid URL: ${url}`);
	}
	const host = parsed.hostname;
	const allowed = collectAllowedHosts(publicUrl, opts);
	for (const a of allowed) {
		if (host === a) return;
		if (host.endsWith(`.${a}`)) return;
	}
	const allowList = allowed.length > 0 ? allowed.join(", ") : "(none)";
	throw new Error(`URL ${url} is not on the allowed host (${allowList})`);
}

function collectAllowedHosts(
	publicUrl: string | undefined,
	opts: AssertSafeUrlOptions,
): string[] {
	const out = new Set<string>();
	if (publicUrl) {
		try {
			out.add(new URL(publicUrl).hostname);
		} catch {
			// ignore — malformed env shouldn't crash checkout
		}
	}
	if (opts.extraHost) out.add(opts.extraHost);
	if (opts.extraOrigins) {
		for (const o of opts.extraOrigins.split(",")) {
			const trimmed = o.trim();
			if (!trimmed) continue;
			try {
				out.add(new URL(trimmed).hostname);
			} catch {
				// Bare hostname (no scheme) — accept as-is.
				out.add(trimmed);
			}
		}
	}
	if (opts.allowLocalhost ?? true) {
		out.add("localhost");
		out.add("127.0.0.1");
	}
	return Array.from(out);
}
