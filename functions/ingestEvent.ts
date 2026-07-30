import { action } from "@pylonsync/functions";

/**
 * Revtrail first-party beacon relay — POST /api/fn/ingestEvent.
 *
 * Ad-blocker-proof analytics. The tracking snippet (see web/app/layout.tsx)
 * loads `/rt/track.js` from THIS origin (a vendored first-party copy of
 * Revtrail's script), and that script derives its beacon endpoint as
 * `new URL(script.src).origin + "/api/fn/ingestEvent"` — i.e. right here.
 * Because both the script and the beacon are same-origin, host-based
 * blocklists (uBlock/EasyPrivacy) never see `revtrail.pyln.dev` and can't
 * drop the pageviews/conversions. This handler forwards the beacon to
 * Revtrail server-side.
 *
 * Why a raw pass-through (not a typed action with `args`): the browser reads
 * the JSON response back (`{ ok, visitorId }`) on the FIRST beacon to learn
 * its server-computed visitor id, which then rides on Stripe Checkout as
 * `client_reference_id` for first-touch revenue attribution. Pylon returns an
 * action's return value as the raw top-level JSON body (verified against
 * Revtrail's own Pylon `/api/fn/ingestEvent`), so returning Revtrail's parsed
 * response verbatim reproduces exactly what the snippet expects. We read the
 * exact bytes off `ctx.request.rawBody` (same pattern as stripeWebhook) rather
 * than declaring `args`, so any future field the snippet adds passes through
 * untouched.
 *
 * Geo/UA: Revtrail can't see the real visitor once we relay — the TCP peer is
 * our Fly machine. We forward the visitor's `user-agent` and `x-forwarded-for`
 * (its first entry is the visitor IP; Cloudflare appends, never replaces, so
 * Revtrail can IP-geolocate) plus `cf-ipcountry`. pylonsync.com sits behind
 * Cloudflare, so `cf-ipcountry` is already populated on the inbound request —
 * the direct analog of the Vercel `x-vercel-ip-country` shim. (Revtrail's own
 * Cloudflare may overwrite `cf-ipcountry` on ingress from us, which is exactly
 * why `x-forwarded-for` is the dependable channel.)
 *
 * SECURITY: `site` is a PUBLIC key and cannot authenticate money — this relay
 * only forwards what the browser sends (event pings), never revenueCents /
 * currency / customerId. Verified revenue arrives through Stripe's signed
 * webhook -> Revtrail `stripeRevenue`, not here.
 */
export default action({
	// No `args`: we forward the raw request body verbatim. Pylon does not
	// validate the body against the (empty) arg schema, so arbitrary beacon
	// shapes pass through — same as stripeWebhook / githubWebhook.
	args: {},
	// The browser calls this with no session cookie. The framework's default
	// `auth: "user"` floor would 401 every beacon before the handler runs.
	// There is nothing to authenticate here — it's a public analytics relay.
	auth: "public",
	async handler(ctx) {
		if (!ctx.request) {
			throw ctx.error("BAD_INVOCATION", "ingestEvent requires HTTP context");
		}

		const base = (ctx.env.REVTRAIL_BASE_URL ?? "https://revtrail.pyln.dev").replace(
			/\/$/,
			"",
		);
		const h = ctx.request.headers; // header names are lowercased by the runtime

		const forwarded: Record<string, string> = {
			"content-type": "application/json",
		};
		if (h["user-agent"]) forwarded["user-agent"] = h["user-agent"];
		if (h["x-forwarded-for"]) forwarded["x-forwarded-for"] = h["x-forwarded-for"];
		if (h["cf-ipcountry"]) forwarded["cf-ipcountry"] = h["cf-ipcountry"];

		let res: Response;
		try {
			res = await fetch(`${base}/api/fn/ingestEvent`, {
				method: "POST",
				headers: forwarded,
				body: ctx.request.rawBody,
			});
		} catch {
			// Revtrail unreachable. Analytics is strictly best-effort and must
			// never surface as an error to the page — the snippet treats a
			// failed beacon as "no visitor id yet" and moves on.
			return { ok: false };
		}

		const text = await res.text();
		try {
			// Pass Revtrail's response through verbatim (top-level `visitorId`
			// is what the snippet reads to seed first-touch attribution).
			return JSON.parse(text) as Record<string, unknown>;
		} catch {
			return { ok: res.ok };
		}
	},
});
