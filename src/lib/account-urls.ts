// Where "Create your account" and "Sign in" go.
//
// These marketing components are compiled into TWO apps that answer on two
// different hosts:
//
//   apps/control-plane   → www.usesmallware.com   (the product; owns /signup)
//   apps/pylonsync-site  → www.pylonsync.com      (the framework; no auth at all)
//
// Every auth link was a relative path, which is only correct in the first one.
// On pylonsync.com the header's "Create your account →", the two pricing
// buttons, and every in-page CTA pointed at /signup and /login on a host that
// serves neither — four dead 404s on the primary conversion path, plus more
// down the page.
//
// So the account host is named absolutely. From pylonsync.com that crosses to
// the product; from usesmallware.com it resolves to the same origin the visitor
// is already on. The cost is a full navigation instead of client-side routing
// on the control plane's own marketing page, which is the correct trade against
// links that silently 404 on the other domain.
//
// Signed-in state stays the caller's business: pylonsync.com has no session, so
// `signedIn` is always false there, and the dashboard link is only ever
// rendered on the host that can answer it.
export const ACCOUNT_ORIGIN = "https://www.usesmallware.com";

/**
 * Absolute URL on the account host. Pass a root-relative path.
 *
 * `accountUrl("/signup?plan=team")` → `https://www.usesmallware.com/signup?plan=team`
 */
export function accountUrl(path: string): string {
	return `${ACCOUNT_ORIGIN}${path.startsWith("/") ? path : `/${path}`}`;
}

/** Where a CTA should go given whether the visitor is signed in. */
export function ctaUrl(signedIn: boolean, signupPath = "/signup"): string {
	return accountUrl(signedIn ? "/dashboard" : signupPath);
}
