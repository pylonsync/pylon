"use client";

/**
 * Thrown by the API client on any non-2xx response. Carries the wire
 * `code` (e.g. `OAUTH_INVALID_STATE`) so UI can branch on specific
 * failures instead of string-matching the message.
 */
export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string,
	) {
		super(message);
	}
}

/**
 * Options for {@link createPylonClient}.
 */
export type CreatePylonClientOptions = {
	/**
	 * Base URL prefixed to every request. Empty string (default) means
	 * same-origin — the right choice when Next is proxying `/api/*`
	 * to the Pylon backend via `next.config.ts` rewrites, because the
	 * browser sees same-origin and the cookie is auto-attached.
	 */
	baseUrl?: string;
};

/**
 * Build a typed `api()` function for the dashboard. Sends `credentials:
 * "include"` so the HttpOnly Pylon session cookie rides along on every
 * request — there's no token in JS to steal.
 *
 * ```ts
 * import { createPylonClient } from "@pylonsync/next/client";
 * export const api = createPylonClient();
 *
 * const me = await api<Me>("/api/entities/User/abc");
 * ```
 *
 * Throws {@link ApiError} on non-2xx so callers can `instanceof` it
 * and surface `.code` + `.message` near the form that triggered the
 * call.
 */
export function createPylonClient(opts: CreatePylonClientOptions = {}) {
	const baseUrl = opts.baseUrl ?? "";
	return async function api<T = unknown>(
		path: string,
		init: RequestInit = {},
	): Promise<T> {
		const headers: Record<string, string> = {
			"Content-Type": "application/json",
			...((init.headers as Record<string, string>) ?? {}),
		};
		const res = await fetch(`${baseUrl}${path}`, {
			...init,
			headers,
			credentials: "include",
		});
		const text = await res.text();
		// 204 / empty body → null. Callers that know the endpoint
		// returns nothing don't have to special-case.
		const body = text ? JSON.parse(text) : null;
		if (!res.ok) {
			const code = body?.error?.code ?? body?.code ?? "UNKNOWN";
			const msg = body?.error?.message ?? body?.message ?? res.statusText;
			throw new ApiError(res.status, code, msg);
		}
		return body as T;
	};
}

/**
 * Default client: same-origin, paired with the Next.js `/api/*`
 * rewrite. Most apps use this directly; call {@link createPylonClient}
 * if you need a different `baseUrl`.
 */
export const api = createPylonClient();
