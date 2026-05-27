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

export interface OrgMember {
	user_id: string;
	role: string;
	joined_at: number;
}

export async function listOrgMembers(orgId: string): Promise<OrgMember[]> {
	try {
		return await get<OrgMember[]>(`/api/auth/orgs/${orgId}/members`);
	} catch {
		return [];
	}
}

export async function updateMemberRole(
	orgId: string,
	userId: string,
	role: string,
): Promise<{ updated: boolean }> {
	return req<{ updated: boolean }>(
		"PUT",
		`/api/auth/orgs/${orgId}/members/${userId}`,
		{ role },
	);
}

export async function removeMember(
	orgId: string,
	userId: string,
): Promise<{ removed: boolean }> {
	return req<{ removed: boolean }>(
		"DELETE",
		`/api/auth/orgs/${orgId}/members/${userId}`,
	);
}

export interface PendingInvite {
	id: string;
	email: string;
	role: string;
	token_prefix: string;
	invited_by: string;
	created_at: number;
	expires_at: number;
}

export interface InviteResult {
	id: string;
	email: string;
	role: string;
	expires_at: number;
	accept_url: string;
	/** Dev mode only — full token so the inviter can copy/paste when
	 *  the email transport isn't configured. */
	token?: string;
}

export async function listInvites(orgId: string): Promise<PendingInvite[]> {
	try {
		return await get<PendingInvite[]>(`/api/auth/orgs/${orgId}/invites`);
	} catch {
		return [];
	}
}

export async function createInvite(
	orgId: string,
	email: string,
	role: string,
): Promise<InviteResult> {
	return post<InviteResult>(`/api/auth/orgs/${orgId}/invites`, { email, role });
}

export async function revokeInvite(
	orgId: string,
	inviteId: string,
): Promise<{ revoked: boolean }> {
	return req<{ revoked: boolean }>(
		"DELETE",
		`/api/auth/orgs/${orgId}/invites/${inviteId}`,
	);
}

export async function acceptInvite(
	token: string,
): Promise<{ org_id: string; role: string }> {
	return post<{ org_id: string; role: string }>(
		`/api/auth/invites/${encodeURIComponent(token)}/accept`,
		{},
	);
}

export interface ConnectionAuthUrl {
	url: string;
}

export async function connectionAuthUrl(
	name: string,
	postRedirect?: string,
): Promise<ConnectionAuthUrl> {
	return post<ConnectionAuthUrl>(
		`/api/connections/${encodeURIComponent(name)}/auth-url`,
		postRedirect ? { post_redirect: postRedirect } : {},
	);
}

export interface ActiveSession {
	token_prefix: string;
	user_id: string;
	device?: string;
	created_at: number;
	expires_at: number;
}

export async function listActiveSessions(): Promise<ActiveSession[]> {
	try {
		return await get<ActiveSession[]>("/api/auth/sessions");
	} catch {
		return [];
	}
}

export async function revokeAllSessions(): Promise<{ revoked_count: number }> {
	return req<{ revoked_count: number }>("DELETE", "/api/auth/sessions");
}

export async function changePassword(input: {
	currentPassword: string;
	newPassword: string;
}): Promise<{ ok: true }> {
	return post("/api/auth/password/change", input);
}

export async function requestPasswordReset(
	email: string,
): Promise<{ sent: true }> {
	return post("/api/auth/password/reset/request", { email });
}

export async function completePasswordReset(input: {
	token: string;
	newPassword: string;
}): Promise<{ ok: true }> {
	return post("/api/auth/password/reset/complete", input);
}

export interface ApiKeySummary {
	id: string;
	prefix: string;
	name: string;
	scopes?: string | null;
	expires_at?: number | null;
	last_used_at?: number | null;
	created_at: number;
}

export interface ApiKeyCreated extends ApiKeySummary {
	/** Shown once on creation. Never returned again. */
	key: string;
}

export async function listApiKeys(): Promise<ApiKeySummary[]> {
	try {
		return await get<ApiKeySummary[]>("/api/auth/api-keys");
	} catch {
		return [];
	}
}

export async function createApiKey(input: {
	name: string;
	scopes?: string;
	expiresAt?: number;
}): Promise<ApiKeyCreated> {
	return post<ApiKeyCreated>("/api/auth/api-keys", {
		name: input.name,
		scopes: input.scopes,
		expires_at: input.expiresAt,
	});
}

export async function revokeApiKey(
	id: string,
): Promise<{ revoked: boolean }> {
	return req<{ revoked: boolean }>(
		"DELETE",
		`/api/auth/api-keys/${encodeURIComponent(id)}`,
	);
}

async function req<T>(
	method: "PUT" | "DELETE",
	path: string,
	body?: unknown,
): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method,
		credentials: "include",
		headers: body ? { "content-type": "application/json" } : undefined,
		body: body ? JSON.stringify(body) : undefined,
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

export { ApiError };
