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

// Turn a non-2xx Response into an ApiError, pulling the stable error `code` and
// human `message` out of Pylon's error envelope. The server returns
// `{ "error": { "code": "...", "message": "..." } }`; older/other endpoints may
// send a flat `{ "error": "CODE", "message": "..." }`. Handle both so callers
// always get a real `.code` to branch on (not "Bad Request").
async function throwApiError(res: Response): Promise<never> {
	const payload = (await res.json().catch(() => ({}))) as {
		error?: { code?: string; message?: string } | string;
		code?: string;
		message?: string;
	};
	const nested =
		payload && typeof payload.error === "object" ? payload.error : null;
	const code =
		nested?.code ??
		(typeof payload?.error === "string" ? payload.error : undefined) ??
		payload?.code ??
		`HTTP_${res.status}`;
	const message =
		nested?.message ?? payload?.message ?? res.statusText ?? "request failed";
	throw new ApiError(code, message, res.status);
}

async function post<T>(path: string, body: unknown): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method: "POST",
		credentials: "include",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(body),
	});
	if (!res.ok) await throwApiError(res);
	return res.json() as Promise<T>;
}

async function get<T>(path: string): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method: "GET",
		credentials: "include",
	});
	if (!res.ok) await throwApiError(res);
	return res.json() as Promise<T>;
}

/**
 * Mint a guest session if the browser doesn't already have a Pylon
 * token. Idempotent — calling repeatedly is a no-op once a session
 * exists. Designed for zero-auth demos (`<EnsureGuest>` is the
 * component wrapper); apps that need real users should use `<SignIn>`
 * instead.
 *
 * Each browser gets its own anonymous `user_id`, so multi-tab demos
 * still observe live sync across tabs sharing the same guest. To
 * reset (e.g. "switch identity"), clear the token from storage and
 * call this again.
 */
export async function ensureGuestSession(): Promise<SessionResponse | null> {
	if (typeof window === "undefined") return null;
	const existing = window.localStorage?.getItem(storageKey("token"));
	if (existing) return null;
	try {
		const res = await fetch(`${getBaseUrl()}/api/auth/guest`, {
			method: "POST",
			credentials: "include",
		});
		if (!res.ok) return null;
		const body = (await res.json()) as SessionResponse;
		persistSession(body);
		return body;
	} catch {
		return null;
	}
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

/** Rename an org. Server gates this to owners + admins (`can_manage_members`). */
export async function renameOrg(
	orgId: string,
	name: string,
): Promise<{ id: string; name: string }> {
	return req<{ id: string; name: string }>("PATCH", `/api/auth/orgs/${orgId}`, {
		name,
	});
}

/** Delete an org and its memberships + invites. Server gates this to owners
 *  (`can_delete_org`). Irreversible. */
export async function deleteOrg(orgId: string): Promise<{ deleted: boolean }> {
	return req<{ deleted: boolean }>("DELETE", `/api/auth/orgs/${orgId}`);
}

export interface OrgMember {
	user_id: string;
	role: string;
	joined_at: number;
	/** Member's email, joined server-side from the User entity. `null` if the
	 *  User row is missing or has no email. */
	email: string | null;
	/** Member's display name (displayName / name / fullName), if set. */
	name: string | null;
	/** OAuth profile picture from the member's most recent provider login.
	 *  `null` for password/magic-link users — render an initials fallback.
	 *  Absent on servers older than 0.4.2. */
	avatar_url?: string | null;
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
): Promise<{ updated: boolean; role: string }> {
	return req<{ updated: boolean; role: string }>(
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
	method: "PUT" | "PATCH" | "DELETE",
	path: string,
	body?: unknown,
): Promise<T> {
	const res = await fetch(`${getBaseUrl()}${path}`, {
		method,
		credentials: "include",
		headers: body ? { "content-type": "application/json" } : undefined,
		body: body ? JSON.stringify(body) : undefined,
	});
	if (!res.ok) await throwApiError(res);
	return res.json() as Promise<T>;
}

export { ApiError };
