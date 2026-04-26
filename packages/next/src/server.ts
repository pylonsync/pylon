import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import type { OAuthProvider } from "./auth";

/**
 * Resolved Pylon session for a server-side request. Use `cookieHeader`
 * when forwarding the cookie to subsequent Pylon API calls (see
 * {@link pylonFetch}).
 */
export type PylonSession = {
	userId: string;
	cookieHeader: string;
};

/**
 * Full auth shape from `/api/auth/me`. Superset of {@link PylonSession}
 * — adds the active tenant and admin flag. Use when the server-rendered
 * UI needs to branch on tenant or role (e.g. show org switcher only
 * when isAdmin, scope queries to tenantId).
 */
export type PylonAuth = {
	userId: string;
	tenantId: string | null;
	isAdmin: boolean;
	cookieHeader: string;
};

/**
 * Options for the server-side helpers. Both default to env vars:
 * `PYLON_COOKIE_NAME` (fallback `pylon_session`) and `PYLON_TARGET`
 * (fallback `http://localhost:4321` in dev only — required in prod).
 */
export type SessionOptions = {
	cookieName?: string;
	target?: string;
};

function resolveOpts(opts: SessionOptions = {}) {
	return {
		cookieName:
			opts.cookieName ?? process.env.PYLON_COOKIE_NAME ?? "pylon_session",
		target: opts.target ?? resolveTarget(),
	};
}

/**
 * Resolve PYLON_TARGET with a prod-safe default. The localhost
 * fallback exists for `next dev` ergonomics — in production it would
 * silently route every server-side Pylon call to whatever is listening
 * on localhost:4321 (sidecar, debug shim, nothing). Throw loudly so
 * the misconfiguration surfaces immediately on the first request.
 */
function resolveTarget(): string {
	const env = process.env.PYLON_TARGET;
	if (env && env.length > 0) return env;
	if (process.env.NODE_ENV === "production") {
		throw new Error(
			"PYLON_TARGET is required in production. Set it to your Pylon control-plane origin (e.g. https://api.example.com).",
		);
	}
	return "http://localhost:4321";
}

/**
 * Read the Pylon session cookie and validate it server-side via
 * `/api/auth/me`. Returns `null` if the cookie is missing or the
 * session has been revoked / expired. Suitable for layouts that want
 * to render different UI for anonymous vs. authenticated users.
 *
 * Use {@link requirePylonSession} if you'd rather just redirect.
 */
export async function getPylonSession(
	opts?: SessionOptions,
): Promise<PylonSession | null> {
	const { cookieName, target } = resolveOpts(opts);
	const cookieStore = await cookies();
	const session = cookieStore.get(cookieName);
	if (!session) return null;

	const cookieHeader = `${cookieName}=${session.value}`;
	const auth = await fetch(`${target}/api/auth/me`, {
		headers: { cookie: cookieHeader },
		cache: "no-store",
	})
		.then((r) => r.json() as Promise<{ user_id?: string }>)
		.catch(() => ({}) as { user_id?: string });
	if (!auth.user_id) return null;
	return { userId: auth.user_id, cookieHeader };
}

/**
 * Like {@link getPylonSession} but redirects to `loginUrl` (default
 * `/login`) if the session is missing or invalid. Use in Server
 * Component layouts to gate a whole subtree without leaking protected
 * UI before the redirect.
 *
 * ```ts
 * export default async function DashboardLayout({ children }) {
 *   const { userId, cookieHeader } = await requirePylonSession();
 *   const me = await fetchMe(userId, cookieHeader);
 *   return <Chrome user={me}>{children}</Chrome>;
 * }
 * ```
 */
export async function requirePylonSession(
	opts?: SessionOptions & { loginUrl?: string },
): Promise<PylonSession> {
	const session = await getPylonSession(opts);
	if (!session) redirect(opts?.loginUrl ?? "/login");
	return session;
}

/**
 * Like {@link getPylonSession} but returns the full auth shape —
 * userId + tenantId + isAdmin. Use when the server-rendered UI
 * needs more than "is there any session" (e.g. scoping a query to
 * the active tenant, showing an admin-only menu).
 *
 * Returns `null` if no session cookie is present, or the cookie's
 * session has been revoked / expired.
 *
 * ```ts
 * const auth = await getAuth();
 * if (!auth) redirect("/login");
 * if (!auth.tenantId) redirect("/onboarding");
 * ```
 */
export async function getAuth(
	opts?: SessionOptions,
): Promise<PylonAuth | null> {
	const { cookieName, target } = resolveOpts(opts);
	const cookieStore = await cookies();
	const session = cookieStore.get(cookieName);
	if (!session) return null;

	const cookieHeader = `${cookieName}=${session.value}`;
	const auth = await fetch(`${target}/api/auth/me`, {
		headers: { cookie: cookieHeader },
		cache: "no-store",
	})
		.then(
			(r) =>
				r.json() as Promise<{
					user_id?: string;
					tenant_id?: string | null;
					is_admin?: boolean;
				}>,
		)
		.catch(
			() =>
				({}) as {
					user_id?: string;
					tenant_id?: string | null;
					is_admin?: boolean;
				},
		);
	if (!auth.user_id) return null;
	return {
		userId: auth.user_id,
		tenantId: auth.tenant_id ?? null,
		isAdmin: auth.is_admin ?? false,
		cookieHeader,
	};
}

/**
 * Like {@link getAuth} but redirects to `loginUrl` (default `/login`)
 * if no session. The non-null return type frees layouts from the
 * `if (!auth) redirect(...)` guard.
 */
export async function requireAuth(
	opts?: SessionOptions & { loginUrl?: string },
): Promise<PylonAuth> {
	const auth = await getAuth(opts);
	if (!auth) redirect(opts?.loginUrl ?? "/login");
	return auth;
}

/**
 * Fetch the authed user's row from the User entity in addition to
 * resolving auth. Eliminates the "header chrome renders empty for a
 * frame, then the username pops in" flicker on dashboard layouts.
 *
 * The User shape is app-defined (different Pylon apps add their own
 * fields beyond the base `email`/`displayName`). Pass your `User`
 * type as the generic so the return value is correctly shaped.
 *
 * ```ts
 * type User = { id: string; email: string; displayName: string };
 *
 * const me = await getCurrentUser<User>();
 * if (!me) redirect("/login");
 * return <Chrome user={me.user} />;
 * ```
 *
 * Returns `null` when there's no session OR the user row can't be
 * loaded (deleted account, transient API failure). Most layouts
 * should treat both cases as "redirect to login" — see
 * {@link requireCurrentUser}.
 */
export async function getCurrentUser<U = Record<string, unknown>>(
	opts?: SessionOptions,
): Promise<{ auth: PylonAuth; user: U } | null> {
	const auth = await getAuth(opts);
	if (!auth) return null;
	const { target } = resolveOpts(opts);
	const res = await fetch(
		`${target}/api/entities/User/${encodeURIComponent(auth.userId)}`,
		{ headers: { cookie: auth.cookieHeader }, cache: "no-store" },
	);
	if (!res.ok) return null;
	const user = (await res.json()) as U;
	return { auth, user };
}

/**
 * Like {@link getCurrentUser} but redirects to `loginUrl` (default
 * `/login`) if the session or user can't be resolved.
 */
export async function requireCurrentUser<U = Record<string, unknown>>(
	opts?: SessionOptions & { loginUrl?: string },
): Promise<{ auth: PylonAuth; user: U }> {
	const me = await getCurrentUser<U>(opts);
	if (!me) redirect(opts?.loginUrl ?? "/login");
	return me;
}

/**
 * Server-side fetch of the enabled OAuth providers. Use in Server
 * Components for /login and /signup so the "Continue with Google"
 * row paints in the initial HTML — no post-mount flicker like the
 * client-side {@link useOAuthProviders} hook causes.
 *
 * Returns an empty array on any failure (control plane unreachable,
 * 5xx, etc.) so the page can fall back to rendering the password
 * form alone instead of crashing.
 *
 * ```tsx
 * // app/login/page.tsx
 * import { getOAuthProviders } from "@pylonsync/next/server";
 * import { LoginForm } from "./login-form"; // "use client"
 *
 * export default async function LoginPage() {
 *   const providers = await getOAuthProviders();
 *   return <LoginForm providers={providers} />;
 * }
 * ```
 *
 * Hits PYLON_TARGET directly rather than going through the Next
 * /api/* rewrite — the rewrite is a browser-side same-origin
 * optimization, irrelevant on the server, and skipping it avoids a
 * pointless localhost→localhost hop in dev + a real network round
 * trip to ourselves in prod.
 */
export async function getOAuthProviders(
	opts?: Pick<SessionOptions, "target">,
): Promise<OAuthProvider[]> {
	const { target } = resolveOpts(opts);
	try {
		const res = await fetch(`${target}/api/auth/providers`, {
			// Providers are env-derived on the control plane (set when
			// PYLON_OAUTH_*_CLIENT_ID is configured). They don't change
			// per-request, but they DO change across deploys. no-store
			// is the safest default until callers explicitly opt in to
			// caching via revalidate.
			cache: "no-store",
		});
		if (!res.ok) return [];
		return (await res.json()) as OAuthProvider[];
	} catch {
		return [];
	}
}

/**
 * Server-side fetch to the Pylon control plane that auto-forwards the
 * caller's session cookie. Use from Server Components, Route Handlers,
 * and Server Actions to call Pylon as the user.
 *
 * ```ts
 * const me: Me = await pylonFetch(`/api/entities/User/${userId}`)
 *   .then(r => r.json());
 * ```
 *
 * Defaults to `cache: "no-store"` because Pylon responses are
 * per-user; pass an explicit `cache` to override.
 */
export async function pylonFetch(
	path: string,
	init: RequestInit = {},
	opts?: SessionOptions,
): Promise<Response> {
	const { cookieName, target } = resolveOpts(opts);
	const cookieStore = await cookies();
	const session = cookieStore.get(cookieName);
	const headers = new Headers(init.headers);
	if (session) headers.set("cookie", `${cookieName}=${session.value}`);
	return fetch(`${target}${path}`, {
		cache: "no-store",
		...init,
		headers,
	});
}
