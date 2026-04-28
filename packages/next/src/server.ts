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
 */
export type PylonServerConfig = {
	cookieName: string;
	target?: string;
	getMeFn?: string;
	loginUrl?: string;
};

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

	const target = (): string => resolveTarget(targetOpt);

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
		const auth = await fetch(`${target()}/api/auth/me`, {
			headers: { cookie: session.header },
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
		return fetch(`${target()}${path}`, {
			cache: "no-store",
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
		try {
			const res = await fetch(`${target()}/api/auth/providers`, {
				cache: "no-store",
			});
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
		const auth = await getAuth();
		if (!auth) return null;
		try {
			const user = await pylonJsonBound<U>(`/api/fn/${getMeFn}`, {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: "{}",
			});
			if (user == null) return null;
			return { auth, user };
		} catch {
			// Function may not be registered yet, or the row was
			// deleted while logged in — treat as anonymous.
			return null;
		}
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
} = {}): Promise<OAuthProvider[]> {
	const target = resolveTarget(opts.target);
	try {
		const res = await fetch(`${target}/api/auth/providers`, {
			cache: "no-store",
		});
		if (!res.ok) return [];
		return (await res.json()) as OAuthProvider[];
	} catch {
		return [];
	}
}
