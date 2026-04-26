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
	 * Routes the proxy applies to. Forms the `config.matcher` Next reads.
	 * Defaults to `["/dashboard/:path*"]`.
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

	function proxy(request: NextRequest) {
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
