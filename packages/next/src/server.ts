import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import type { OAuthProvider } from "./auth";
import { ApiError } from "./errors";

/**
 * Full auth shape from `/api/auth/me`. Use when the server-rendered
 * UI needs more than "is there any session" — branching on tenant or
 * role, scoping a query to the active tenant, etc.
 */
export type PylonAuth = {
	userId: string;
	tenantId: string | null;
	isAdmin: boolean;
	cookieHeader: string;
};

/**
 * Configuration for {@link createPylonServer}.
 *
 * `cookieName` is REQUIRED — there's no safe default because Pylon's
 * binary emits `${app_name}_session` (e.g. `pylon-cloud_session`) and
 * the package can't know your app name. Passing it explicitly here
 * also kills a class of bugs where a wrong env var silently breaks
 * auth in production.
 *
 * `target` is the Pylon control-plane origin. Defaults to the
 * `PYLON_TARGET` env var; throws in production if unset.
 *
 * `getMeFn` is the server function name used by {@link PylonServer.getMe}.
 * Default `"getMe"` — most apps just declare a `functions/getMe.ts`
 * that returns the current user's safe-to-display fields, see the
 * Pylon Cloud reference for an example.
 *
 * `loginUrl` is where {@link PylonServer.requireAuth} / {@link
 * PylonServer.requireMe} redirect when the session is missing.
 *
 * `timeoutMs` caps every server-side fetch to the control plane.
 * Default 10s. The point is to fail loudly during a backend hiccup
 * (machine restart, Fly proxy stall) rather than letting the React
 * server render block until Vercel's 30s edge timeout. Pair with a
 * route-level `error.tsx` so users see "server unavailable, retry"
 * instead of a blank-tab hang.
 */
export type PylonServerConfig = {
	cookieName: string;
	target?: string;
	getMeFn?: string;
	loginUrl?: string;
	timeoutMs?: number;
};

/**
 * Thrown by `createPylonServer` helpers when the control plane is
 * unreachable — network error, fetch timeout, or any non-HTTP error.
 * Distinct from `ApiError` (which carries an HTTP status from a
 * response that DID come back). `requireMe`/`requireAuth` rethrow this
 * instead of redirecting to /login so the route's `error.tsx` can
 * render a "server unavailable, retry" UI without misleadingly
 * suggesting the user signed out.
 */
export class PylonUnreachableError extends Error {
	readonly cause?: unknown;
	constructor(message: string, cause?: unknown) {
		super(message);
		this.name = "PylonUnreachableError";
		this.cause = cause;
	}
}

/**
 * Bound server helpers — built once per app via {@link createPylonServer}
 * and used everywhere. Eliminates the per-call cookieName / target
 * plumbing the standalone helpers required.
 *
 * ```ts
 * // src/lib/pylon.ts
 * export const pylon = createPylonServer({
 *   cookieName: "myapp_session",
 *   getMeFn: "getMe",
 * });
 *
 * // src/app/dashboard/layout.tsx
 * import { pylon } from "@/lib/pylon";
 * const me = await pylon.requireMe<User>();
 * const orgs = await pylon.json<Org[]>("/api/entities/Organization");
 * ```
 */
export interface PylonServer {
	/** Forwarded raw fetch — caller handles status + body parsing. */
	fetch(path: string, init?: RequestInit): Promise<Response>;
	/**
	 * Fetch + parse + status check in one. Throws {@link ApiError} on
	 * non-2xx so callers don't have to write the `if (!res.ok)` dance
	 * before every `.json()`.
	 */
	json<T = unknown>(path: string, init?: RequestInit): Promise<T>;
	/** Resolved auth + null on no session. */
	getAuth(): Promise<PylonAuth | null>;
	/** Resolved auth, or `redirect()` to `loginUrl`. */
	requireAuth(): Promise<PylonAuth>;
	/**
	 * OAuth provider list, server-side. Eliminates the post-mount
	 * flicker the client `useOAuthProviders` causes.
	 */
	getOAuthProviders(): Promise<OAuthProvider[]>;
	/**
	 * Current user (auth + the row your `getMe` function returns).
	 * Calls `/api/fn/${getMeFn}` rather than the entity API — the
	 * function bypasses entity policies and lets you control the
	 * projection (typically: id, email, displayName; never
	 * passwordHash).
	 */
	getMe<U = Record<string, unknown>>(): Promise<{
		auth: PylonAuth;
		user: U;
	} | null>;
	/** Like `getMe`, redirects to `loginUrl` on null. */
	requireMe<U = Record<string, unknown>>(): Promise<{
		auth: PylonAuth;
		user: U;
	}>;
}

/**
 * Build a server-side Pylon helper, bound to one app's configuration.
 * One factory call per app, no per-call boilerplate.
 *
 * See {@link PylonServerConfig} for the required options.
 */
export function createPylonServer(config: PylonServerConfig): PylonServer {
	const cookieName = config.cookieName;
	const targetOpt = config.target;
	const getMeFn = config.getMeFn ?? "getMe";
	const loginUrl = config.loginUrl ?? "/login";
	const timeoutMs = config.timeoutMs ?? 10_000;

	const target = (): string => resolveTarget(targetOpt);

	/**
	 * Wrap a control-plane fetch with the configured timeout, normalizing
	 * any non-HTTP failure (abort, DNS, connection-refused, mid-stream
	 * disconnect during a machine restart) into {@link PylonUnreachableError}.
	 * Callers that need to differentiate "server hiccup" from "real 4xx"
	 * key off the error type.
	 */
	async function fetchWithTimeout(
		url: string,
		init: RequestInit = {},
	): Promise<Response> {
		const ac = new AbortController();
		const t = setTimeout(() => ac.abort(), timeoutMs);
		try {
			return await fetch(url, {
				cache: "no-store",
				...init,
				signal: ac.signal,
			});
		} catch (err) {
			if (err instanceof Error && err.name === "AbortError") {
				throw new PylonUnreachableError(
					`Pylon control plane timed out after ${timeoutMs}ms (${url})`,
					err,
				);
			}
			throw new PylonUnreachableError(
				`Pylon control plane unreachable (${url}): ${err instanceof Error ? err.message : String(err)}`,
				err,
			);
		} finally {
			clearTimeout(t);
		}
	}

	async function readSession(): Promise<{
		header: string;
		value: string;
	} | null> {
		const cookieStore = await cookies();
		const c = cookieStore.get(cookieName);
		if (!c) return null;
		return { header: `${cookieName}=${c.value}`, value: c.value };
	}

	async function getAuth(): Promise<PylonAuth | null> {
		const session = await readSession();
		if (!session) return null;
		// Network failures bubble as PylonUnreachableError — DON'T swallow
		// them as null. Older code did `.catch(() => ({}))` which made a
		// hung backend look like an unauthenticated session, so
		// `requireAuth` redirected to /login when the real fix was "retry".
		// The route's `error.tsx` boundary handles the throw correctly.
		const res = await fetchWithTimeout(`${target()}/api/auth/me`, {
			headers: { cookie: session.header },
		});
		const auth = (await res.json()) as {
			user_id?: string;
			tenant_id?: string | null;
			is_admin?: boolean;
		};
		if (!auth.user_id) return null;
		return {
			userId: auth.user_id,
			tenantId: auth.tenant_id ?? null,
			isAdmin: auth.is_admin ?? false,
			cookieHeader: session.header,
		};
	}

	async function requireAuth(): Promise<PylonAuth> {
		const a = await getAuth();
		if (!a) redirect(loginUrl);
		return a;
	}

	async function pylonFetchBound(
		path: string,
		init: RequestInit = {},
	): Promise<Response> {
		const session = await readSession();
		const headers = new Headers(init.headers);
		if (session) headers.set("cookie", session.header);
		return fetchWithTimeout(`${target()}${path}`, {
			...init,
			headers,
		});
	}

	async function pylonJsonBound<T = unknown>(
		path: string,
		init: RequestInit = {},
	): Promise<T> {
		const res = await pylonFetchBound(path, init);
		const text = await res.text();
		const body = text ? JSON.parse(text) : null;
		if (!res.ok) {
			const code = body?.error?.code ?? body?.code ?? "UNKNOWN";
			const msg = body?.error?.message ?? body?.message ?? res.statusText;
			throw new ApiError(res.status, code, msg);
		}
		return body as T;
	}

	async function getOAuthProvidersBound(): Promise<OAuthProvider[]> {
		// Providers are env-derived on the control plane; they don't
		// change per-request but DO change across deploys. no-store is
		// the safe default until a caller opts into caching.
		//
		// This one DOES swallow control-plane unreachability — the
		// providers list is decorative chrome on the /login page and a
		// transient hiccup shouldn't make the whole login surface
		// error-boundary. The form still works (server-side auth
		// endpoints handle their own errors); the user just sees no
		// "Sign in with Google" button for a moment.
		try {
			const res = await fetchWithTimeout(`${target()}/api/auth/providers`);
			if (!res.ok) return [];
			return (await res.json()) as OAuthProvider[];
		} catch {
			return [];
		}
	}

	async function getMe<U = Record<string, unknown>>(): Promise<{
		auth: PylonAuth;
		user: U;
	} | null> {
		// Single round-trip to /api/auth/session — pylon returns
		// `{ session, user }` where `user` is the User row projected
		// to safe fields (passwordHash + framework-internal columns
		// stripped).
		//
		// Network failures throw PylonUnreachableError (caught by the
		// route's error.tsx). Older code swallowed them as null which
		// made `requireMe` redirect to /login during a backend restart
		// even though the user's session was perfectly valid — a
		// "signed out" UX for what was actually a 5-second hiccup.
		const session = await readSession();
		if (!session) return null;
		const res = await fetchWithTimeout(`${target()}/api/auth/session`, {
			headers: { cookie: session.header },
		});
		const resp = (await res.json()) as {
			session?: {
				user_id?: string;
				tenant_id?: string | null;
				is_admin?: boolean;
			};
			user?: U | null;
		};
		if (!resp.session?.user_id || !resp.user) return null;
		return {
			auth: {
				userId: resp.session.user_id,
				tenantId: resp.session.tenant_id ?? null,
				isAdmin: resp.session.is_admin ?? false,
				cookieHeader: session.header,
			},
			user: resp.user,
		};
	}

	async function requireMe<U = Record<string, unknown>>(): Promise<{
		auth: PylonAuth;
		user: U;
	}> {
		const me = await getMe<U>();
		if (!me) redirect(loginUrl);
		return me;
	}

	return {
		fetch: pylonFetchBound,
		json: pylonJsonBound,
		getAuth,
		requireAuth,
		getOAuthProviders: getOAuthProvidersBound,
		getMe,
		requireMe,
	};
}

/**
 * Resolve PYLON_TARGET with a prod-safe default. The localhost
 * fallback exists for `next dev` ergonomics — in production it would
 * silently route every server-side Pylon call to whatever is listening
 * on localhost:4321 (sidecar, debug shim, nothing). Throw loudly so
 * the misconfiguration surfaces immediately on the first request.
 */
function resolveTarget(target?: string): string {
	if (target && target.length > 0) return target;
	const env = process.env.PYLON_TARGET;
	if (env && env.length > 0) return env;
	if (process.env.NODE_ENV === "production") {
		throw new Error(
			"PYLON_TARGET is required in production. Set it to your Pylon control-plane origin (e.g. https://api.example.com).",
		);
	}
	return "http://localhost:4321";
}

// ---------------------------------------------------------------------------
// Standalone helpers — for callers that want one-off invocations without
// instantiating a {@link PylonServer}. Most apps should prefer
// {@link createPylonServer}; these exist for tests, scripts, and
// migrations.
// ---------------------------------------------------------------------------

/**
 * One-shot version of {@link PylonServer.getAuth}. Pass the cookie
 * name explicitly — the package no longer reads PYLON_COOKIE_NAME
 * from env (silently-overridable env-driven config was a footgun in
 * practice).
 */
export async function getAuth(opts: {
	cookieName: string;
	target?: string;
}): Promise<PylonAuth | null> {
	return createPylonServer({
		cookieName: opts.cookieName,
		target: opts.target,
	}).getAuth();
}

/** One-shot version of {@link PylonServer.requireAuth}. */
export async function requireAuth(opts: {
	cookieName: string;
	target?: string;
	loginUrl?: string;
}): Promise<PylonAuth> {
	return createPylonServer(opts).requireAuth();
}

/** One-shot version of {@link PylonServer.fetch}. */
export async function pylonFetch(
	path: string,
	init?: RequestInit,
	opts?: { cookieName: string; target?: string },
): Promise<Response> {
	if (!opts) {
		throw new Error(
			"pylonFetch requires an `opts` argument with `cookieName`. The package no longer reads PYLON_COOKIE_NAME from env to avoid silent breakage from misconfigured envs.",
		);
	}
	return createPylonServer(opts).fetch(path, init);
}

/** One-shot version of {@link PylonServer.json}. */
export async function pylonJson<T = unknown>(
	path: string,
	init?: RequestInit,
	opts?: { cookieName: string; target?: string },
): Promise<T> {
	if (!opts) {
		throw new Error(
			"pylonJson requires an `opts` argument with `cookieName`.",
		);
	}
	return createPylonServer(opts).json<T>(path, init);
}

/**
 * One-shot OAuth provider list. Doesn't need a cookie (the endpoint
 * is public), but does need a `target` resolution.
 */
export async function getOAuthProviders(opts: {
	target?: string;
	timeoutMs?: number;
} = {}): Promise<OAuthProvider[]> {
	const target = resolveTarget(opts.target);
	const ac = new AbortController();
	const t = setTimeout(() => ac.abort(), opts.timeoutMs ?? 10_000);
	try {
		const res = await fetch(`${target}/api/auth/providers`, {
			cache: "no-store",
			signal: ac.signal,
		});
		if (!res.ok) return [];
		return (await res.json()) as OAuthProvider[];
	} catch {
		return [];
	} finally {
		clearTimeout(t);
	}
}
