import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

/**
 * Options for {@link createPylonProxy}.
 */
export type CreatePylonProxyOptions = {
	/**
	 * Cookie name to look for. Defaults to `process.env.PYLON_COOKIE_NAME`,
	 * falling back to `pylon_session`. Should match the Pylon backend's
	 * `PYLON_COOKIE_NAME` (or `${app_name}_session` if unset there).
	 */
	cookieName?: string;
	/**
	 * Where to redirect unauthenticated requests. Defaults to `/login`.
	 * The original path is preserved in `?next=…` so the login page can
	 * bounce the user back after sign-in.
	 */
	loginUrl?: string;
	/**
	 * Routes the proxy applies to. Defaults to `["/dashboard/:path*"]`.
	 *
	 * The returned `config.matcher` is what Next.js statically extracts
	 * at build time to decide which requests run through this proxy.
	 * Pylon ALSO uses this list at runtime: if the consumer's outer
	 * `proxy.ts` exports a broader `config.matcher` than what was passed
	 * here (e.g. to also handle `/login` in their own logic), Pylon's
	 * proxy function falls through with `NextResponse.next()` for any
	 * request whose path doesn't match this option — so a consumer
	 * stacking proxies doesn't accidentally trigger Pylon's
	 * redirect-to-login on routes Pylon was never supposed to gate.
	 *
	 * Before 0.3.96 this option was only used to populate the returned
	 * `config.matcher`; the runtime function ignored it. Consumers with
	 * broader outer matchers hit ERR_TOO_MANY_REDIRECTS when an
	 * unauthenticated user landed on /login (Pylon redirected /login →
	 * /login?next=/login → /login → …).
	 */
	matcher?: string[];
};

/**
 * Build a Next 16 `proxy.ts` that gates routes on the presence of the
 * Pylon session cookie. Drop the result into `src/proxy.ts`:
 *
 * ```ts
 * import { createPylonProxy } from "@pylonsync/next/proxy";
 *
 * const { proxy, config } = createPylonProxy({
 *   matcher: ["/dashboard/:path*", "/onboarding/:path*"],
 * });
 *
 * export { proxy, config };
 * ```
 *
 * If you wrap Pylon's proxy in your own logic (handling e.g. signed-in
 * users hitting `/login`), pass `matcher` so Pylon only gates the
 * paths you actually want gated. Your outer `config.matcher` can be
 * broader; Pylon's runtime function checks each request's path
 * against the option it was constructed with and falls through
 * (`NextResponse.next()`) for anything outside it.
 *
 * The proxy only checks cookie *presence* — a forged value will fail the
 * server-side `/api/auth/me` revalidation in your layout. Its job is to
 * short-circuit the obvious "no session at all" case before any page
 * renders, so users never see protected UI flash before the redirect.
 */
export function createPylonProxy(opts: CreatePylonProxyOptions = {}) {
	const cookieName =
		opts.cookieName ?? process.env.PYLON_COOKIE_NAME ?? "pylon_session";
	const loginUrl = opts.loginUrl ?? "/login";
	const matcher = opts.matcher ?? ["/dashboard/:path*"];
	const matchers = matcher.map(compilePathMatcher);

	function proxy(request: NextRequest) {
		const pathname = request.nextUrl.pathname;

		// Self-loop guard. Without this, a no-session GET to `/login`
		// itself (when the consumer's outer config.matcher includes
		// `/login` — common when they want to handle "signed-in user
		// hits /login" themselves) gets redirected to
		// `/login?next=/login`, the browser follows, same proxy runs,
		// infinite redirect → ERR_TOO_MANY_REDIRECTS in the browser.
		// Surfaced 2026-05-15 by a pylon-cloud-style consumer with a
		// composed proxy.
		if (pathname === loginUrl) return NextResponse.next();

		// Runtime matcher gate. The `matcher` option is now load-bearing
		// (not just documentation for `config.matcher`) — requests
		// outside it fall through. Lets consumers compose Pylon's
		// proxy under a broader outer matcher without accidentally
		// gating routes Pylon was never told about.
		if (matchers.length > 0 && !matchers.some((m) => m(pathname))) {
			return NextResponse.next();
		}

		const session = request.cookies.get(cookieName);
		if (session) return NextResponse.next();

		const url = request.nextUrl.clone();
		url.pathname = loginUrl;
		// Preserve the original destination so the login page can bounce
		// the user back after sign-in (read `?next=`).
		url.searchParams.set("next", request.nextUrl.pathname);
		return NextResponse.redirect(url);
	}

	return { proxy, config: { matcher } };
}

/**
 * Compile a Next.js-style matcher pattern into a predicate that
 * accepts a pathname and returns true on match. Covers the subset
 * Pylon consumers actually use:
 *
 *   - exact:           "/login"             → only "/login"
 *   - segment:         "/api/:slug"         → "/api/x", not "/api/x/y"
 *   - rest (greedy):   "/dashboard/:path*"  → "/dashboard" and any subpath
 *   - rest (>=1):      "/api/:rest+"        → "/api/x", "/api/x/y", NOT "/api"
 *
 * Anything more exotic (regex modifiers, optional groups, multiple
 * params per segment) we punt on — Pylon's docs only show the
 * patterns above, and a stricter parser would silently mis-match
 * fancier strings instead of failing loudly.
 */
function compilePathMatcher(pattern: string): (pathname: string) => boolean {
	// Split into segments and translate each. Leading `/` is implicit.
	const segs = pattern.split("/").filter(Boolean);
	const parts: string[] = ["^"];
	for (const seg of segs) {
		if (seg === ":path*" || /^:\w+\*$/.test(seg)) {
			// Zero-or-more trailing segments. Optional with leading slash.
			parts.push("(?:/.*)?");
		} else if (/^:\w+\+$/.test(seg)) {
			// One-or-more trailing segments.
			parts.push("/.+");
		} else if (/^:\w+$/.test(seg)) {
			// Single segment.
			parts.push("/[^/]+");
		} else {
			// Literal — escape regex metacharacters.
			parts.push("/" + seg.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
		}
	}
	parts.push("/?$");
	const re = new RegExp(parts.join(""));
	return (pathname) => re.test(pathname);
}
