// Types + injected globals + API helper.
//
// The Rust /studio handler injects window.__PYLON_API__ (origin to call)
// and window.__PYLON_MANIFEST__ (the redacted public manifest the
// dashboard renders against). Everything else is fetched at runtime.

export type ManifestField = {
	name: string;
	type: string;
	optional?: boolean;
	unique?: boolean;
};

export type ManifestEntity = {
	name: string;
	fields: ManifestField[];
	indexes?: { name: string; fields: string[]; unique: boolean }[];
	crdt?: boolean;
};

export type ManifestPolicy = {
	name: string;
	entity?: string;
	action?: string;
};

export type ManifestQuery = { name: string; input: ManifestField[] };
export type ManifestAction = { name: string; input: ManifestField[] };
export type ManifestRoute = {
	path: string;
	mode: string;
	query?: string;
	auth?: string;
};

export type Manifest = {
	manifest_version: number;
	name: string;
	version: string;
	entities: ManifestEntity[];
	queries: ManifestQuery[];
	actions: ManifestAction[];
	policies: ManifestPolicy[];
	routes: ManifestRoute[];
};

declare global {
	interface Window {
		__PYLON_API__?: string;
		__PYLON_MANIFEST__?: Manifest;
	}
}

export const API_BASE: string =
	(typeof window !== "undefined" && window.__PYLON_API__) || "";

export const MANIFEST: Manifest =
	(typeof window !== "undefined" && window.__PYLON_MANIFEST__) ||
	({
		manifest_version: 1,
		name: "Pylon",
		version: "0.0.0",
		entities: [],
		queries: [],
		actions: [],
		policies: [],
		routes: [],
	} as Manifest);

const ADMIN_TOKEN_KEY = "pylon-studio-admin-token";

export function getStoredToken(): string | null {
	try {
		return window.localStorage.getItem(ADMIN_TOKEN_KEY);
	} catch {
		return null;
	}
}

export function setStoredToken(token: string | null): void {
	try {
		if (token) window.localStorage.setItem(ADMIN_TOKEN_KEY, token);
		else window.localStorage.removeItem(ADMIN_TOKEN_KEY);
	} catch {
		// localStorage blocked — fine, session won't persist.
	}
}

export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string,
	) {
		super(message);
	}
}

export type ApiOptions = RequestInit & { token?: string | null };

/**
 * Fetch wrapper that forwards the configured admin token (or whatever
 * was passed in `opts.token`) and parses the standard Pylon error
 * envelope into an ApiError. Returns the parsed JSON body, or `null`
 * for empty 2xx responses.
 */
export async function api<T = unknown>(
	path: string,
	opts: ApiOptions = {},
): Promise<T> {
	const headers: Record<string, string> = {
		"Content-Type": "application/json",
		...((opts.headers as Record<string, string>) ?? {}),
	};
	const token = opts.token ?? getStoredToken();
	if (token) headers.Authorization = `Bearer ${token}`;

	const res = await fetch(`${API_BASE}${path}`, {
		credentials: "include",
		...opts,
		headers,
	});
	const text = await res.text();
	const body = text ? safeJsonParse(text) : null;

	if (!res.ok) {
		const code =
			(body as { error?: { code?: string }; code?: string } | null)?.error
				?.code ??
			(body as { code?: string } | null)?.code ??
			"UNKNOWN";
		const msg =
			(body as { error?: { message?: string }; message?: string } | null)
				?.error?.message ??
			(body as { message?: string } | null)?.message ??
			res.statusText;
		throw new ApiError(res.status, code, msg);
	}
	return body as T;
}

function safeJsonParse(s: string): unknown {
	try {
		return JSON.parse(s);
	} catch {
		return null;
	}
}

export type AuthMe = {
	user_id?: string | null;
	is_admin?: boolean;
	roles?: string[];
	tenant_id?: string | null;
};
