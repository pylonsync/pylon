import { cookies } from "next/headers";
import { redirect } from "next/navigation";

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
 * Options for the server-side helpers. Both default to env vars:
 * `PYLON_COOKIE_NAME` (fallback `pylon_session`) and `PYLON_TARGET`
 * (fallback `http://localhost:4321`).
 */
export type SessionOptions = {
	cookieName?: string;
	target?: string;
};

function resolveOpts(opts: SessionOptions = {}) {
	return {
		cookieName:
			opts.cookieName ?? process.env.PYLON_COOKIE_NAME ?? "pylon_session",
		target:
			opts.target ?? process.env.PYLON_TARGET ?? "http://localhost:4321",
	};
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
