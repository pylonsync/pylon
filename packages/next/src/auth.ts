"use client";

import { useCallback, useEffect, useState } from "react";
import { ApiError, api } from "./client";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type Session = {
	token: string;
	user_id: string;
	expires_at: number;
};

export type OAuthProvider = {
	provider: "google" | "github";
	auth_url: string;
};

// ---------------------------------------------------------------------------
// Direct API helpers
//
// Each one wraps a Pylon `/api/auth/*` endpoint. They're plain async
// functions so callers can compose them however they want — see the
// hooks below for the typical "form submit" pattern.
// ---------------------------------------------------------------------------

export async function signupWithPassword(input: {
	email: string;
	password: string;
	displayName?: string;
}): Promise<Session> {
	return api<Session>("/api/auth/password/register", {
		method: "POST",
		body: JSON.stringify(input),
	});
}

export async function loginWithPassword(input: {
	email: string;
	password: string;
}): Promise<Session> {
	return api<Session>("/api/auth/password/login", {
		method: "POST",
		body: JSON.stringify(input),
	});
}

/**
 * Sign the user out. Best-effort — even if the server-side revoke
 * fails (e.g. session already expired) the clearing Set-Cookie comes
 * back so the browser drops the cookie either way.
 */
export async function logout(): Promise<void> {
	try {
		await api("/api/auth/session", { method: "DELETE" });
	} catch {
		// ignore — token may already be expired/invalid
	}
}

export async function listOAuthProviders(): Promise<OAuthProvider[]> {
	return api<OAuthProvider[]>("/api/auth/providers");
}

/**
 * Kick off an OAuth login. Browser navigates to Pylon's GET login
 * route, which 302s to the provider, which 302s back to Pylon's GET
 * callback (Set-Cookie + 302 to the success URL). The OAuth code
 * never enters JS, so XSS in the dashboard can't intercept the
 * handshake.
 *
 * `successUrl` and `errorUrl` MUST have origins listed in the
 * server's PYLON_TRUSTED_ORIGINS — otherwise pylon's start endpoint
 * 403s with UNTRUSTED_REDIRECT. Defaults are sensible for typical
 * Next.js layouts (current origin's `/dashboard` and `/login`); pass
 * explicit values for apps that route auth elsewhere.
 */
export type StartOAuthLoginOptions = {
	successUrl?: string;
	errorUrl?: string;
};
export function startOAuthLogin(
	provider: OAuthProvider["provider"],
	opts: StartOAuthLoginOptions = {},
): void {
	const origin = window.location.origin;
	const successUrl = opts.successUrl ?? `${origin}/dashboard`;
	const errorUrl = opts.errorUrl ?? `${origin}/login`;
	const params = new URLSearchParams({
		redirect: "1",
		callback: successUrl,
		error_callback: errorUrl,
	});
	window.location.href = `/api/auth/login/${provider}?${params.toString()}`;
}

export async function sendVerificationEmail(): Promise<{
	sent: boolean;
	email: string;
	dev_code?: string;
}> {
	return api("/api/auth/email/send-verification", { method: "POST" });
}

export async function verifyEmail(
	code: string,
): Promise<{ verified: boolean; emailVerified: string }> {
	return api("/api/auth/email/verify", {
		method: "POST",
		body: JSON.stringify({ code }),
	});
}

// ---------------------------------------------------------------------------
// Hooks — form-shaped wrappers that handle the common busy/error state
// ---------------------------------------------------------------------------

/**
 * Fetch the enabled OAuth providers on mount. Returns an empty array
 * until the request resolves, so callers can render the password form
 * unconditionally and conditionally show the OAuth button row when
 * `providers.length > 0`.
 */
export function useOAuthProviders(): OAuthProvider[] {
	const [providers, setProviders] = useState<OAuthProvider[]>([]);
	useEffect(() => {
		listOAuthProviders()
			.then(setProviders)
			.catch(() => {});
	}, []);
	return providers;
}

/**
 * Manage busy + error state for an auth-shaped async submission.
 * Wraps the provided `fn` so callers don't have to write
 * `try/catch/finally` around every form submit.
 *
 * ```tsx
 * const { error, busy, submit } = useAuthSubmit(loginWithPassword);
 *
 * async function onSubmit(e: React.FormEvent) {
 *   e.preventDefault();
 *   const ok = await submit({ email, password });
 *   if (ok) router.push("/dashboard");
 * }
 * ```
 *
 * `submit` returns `undefined` on failure (error is set internally),
 * the awaited resolution otherwise. `setError` is exposed for the
 * rare case a page wants to clear the error programmatically.
 */
export function useAuthSubmit<I, O>(fn: (input: I) => Promise<O>): {
	error: string | null;
	busy: boolean;
	submit: (input: I) => Promise<O | undefined>;
	setError: (e: string | null) => void;
} {
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const submit = useCallback(
		async (input: I): Promise<O | undefined> => {
			setError(null);
			setBusy(true);
			try {
				return await fn(input);
			} catch (err) {
				if (err instanceof ApiError) setError(err.message);
				else setError(err instanceof Error ? err.message : String(err));
				return undefined;
			} finally {
				setBusy(false);
			}
		},
		[fn],
	);
	return { error, busy, submit, setError };
}
