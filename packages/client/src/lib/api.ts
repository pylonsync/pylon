"use client";

import { db, getBaseUrl, storageKey } from "@pylonsync/react";

export interface AuthProvider {
	provider: string;
	auth_url: string;
}

export interface SessionResponse {
	token: string;
	user_id: string;
	expires_at?: number;
}

export interface OrgSummary {
	id: string;
	name: string;
	role: "owner" | "admin" | "member" | string;
	created_at: number;
}

class ApiError extends Error {
	code: string;
	status: number;
	constructor(code: string, message: string, status: number) {
		super(message);
		this.code = code;
		this.status = status;
	}
}

async function post<T>(path: string, body: unknown): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method: "POST",
		credentials: "include",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(body),
	});
	if (!res.ok) {
		const payload = await res.json().catch(() => ({}) as Record<string, unknown>);
		const code = (payload?.error as string) ?? `HTTP_${res.status}`;
		const message =
			(payload?.message as string) ?? res.statusText ?? "request failed";
		throw new ApiError(code, message, res.status);
	}
	return res.json() as Promise<T>;
}

async function get<T>(path: string): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method: "GET",
		credentials: "include",
	});
	if (!res.ok) throw new ApiError(`HTTP_${res.status}`, res.statusText, res.status);
	return res.json() as Promise<T>;
}

export async function listAuthProviders(): Promise<AuthProvider[]> {
	try {
		return await get<AuthProvider[]>("/api/auth/providers");
	} catch {
		// Older binaries may not expose this — render with no OAuth buttons.
		return [];
	}
}

export async function sendMagicLink(
	email: string,
	captchaToken?: string,
): Promise<{ ok: true }> {
	return post("/api/auth/magic/send", { email, captchaToken });
}

export async function verifyMagicLink(
	email: string,
	code: string,
): Promise<SessionResponse> {
	return post("/api/auth/magic/verify", { email, code });
}

export async function passwordRegister(input: {
	email: string;
	password: string;
	displayName?: string;
	captchaToken?: string;
}): Promise<SessionResponse> {
	return post("/api/auth/password/register", input);
}

export async function passwordLogin(input: {
	email: string;
	password: string;
}): Promise<SessionResponse> {
	return post("/api/auth/password/login", input);
}

/**
 * Persist a freshly-minted session locally + tell the sync engine to
 * re-fetch /api/auth/me so cached `useSession()` consumers re-render
 * immediately. Components call this after successful sign-in/sign-up.
 */
export function persistSession(session: SessionResponse): void {
	try {
		if (typeof window !== "undefined" && window.localStorage) {
			window.localStorage.setItem(storageKey("token"), session.token);
		}
	} catch {
		// localStorage can throw (private mode, quota, etc.) — fall through
		// and let the sync engine pick up the token on next start.
	}
	void db.sync.notifySessionChanged();
}

export async function listOrgs(): Promise<OrgSummary[]> {
	try {
		return await get<OrgSummary[]>("/api/auth/orgs");
	} catch {
		return [];
	}
}

export async function createOrg(name: string): Promise<OrgSummary> {
	return post<OrgSummary>("/api/auth/orgs", { name });
}

export { ApiError };
