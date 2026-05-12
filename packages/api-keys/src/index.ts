/**
 * `@pylonsync/api-keys` — per-user / per-org API key issuance, hash
 * storage, scoped permissions, and per-key rate limiting with
 * token-bucket refill.
 *
 * Replaces the dead `api_keys.rs` Rust shell (which was never wired
 * into the runtime). The framework's `api_key_backend.rs` handles
 * the on-the-wire format (`pk_<random>` bearer tokens, constant-
 * time compare); this package layers the user-facing creation +
 * permission scoping + rate-limit metering on top.
 *
 * Key shape:
 *   `pk_<env>_<32 base32 chars>`
 *   `pk_live_X7Y2A9...` — production
 *   `pk_test_...` — explicitly test-keyed
 *
 * Persistence (declared on app side):
 *   ```
 *   entity("ApiKey", {
 *     name: field.string(),
 *     prefix: field.string(),      // first 8 chars, displayable
 *     hash: field.string(),        // SHA-256 of full token + salt
 *     userId: field.string().optional(),
 *     orgId: field.string().optional(),
 *     permissions: field.string(), // JSON array
 *     rateLimitMax: field.number().optional(),
 *     rateLimitWindowSecs: field.number().optional(),
 *     refillTokens: field.number().optional(),  // current bucket
 *     refillUpdatedAt: field.string().optional(),
 *     lastUsedAt: field.string().optional(),
 *     expiresAt: field.string().optional(),
 *     createdAt: field.string(),
 *   })
 *   ```
 *
 * Apps that prefer their own schema customize entity names via
 * `cfg.entities.apiKey`.
 */

import { hashKey, mintRawKey, parseKey, type KeyEnv } from "./keys";
import { checkAndConsume, type RateLimitState } from "./rate-limit";

export type {
	KeyEnv,
} from "./keys";
export type {
	RateLimitState,
	RateLimitDecision,
} from "./rate-limit";
export { hashKey, mintRawKey, parseKey } from "./keys";
export { checkAndConsume } from "./rate-limit";

export interface ApiKeyConfig {
	/**
	 * Env tag used in the key prefix (`pk_live_...` vs `pk_test_...`).
	 * Auto-detected from `PYLON_DEV_MODE` if omitted: `test` in dev,
	 * `live` in prod.
	 */
	env?: KeyEnv;
	/**
	 * Per-key rate limit defaults. Apps can override per-key by
	 * passing the field on creation.
	 */
	defaultRateLimit?: {
		max: number;
		windowSecs: number;
	};
	/**
	 * Token-bucket refill rate (tokens per window). If unset, the
	 * limiter is fixed-window (resets at the boundary). Refill mode
	 * is friendlier — avoids hard-cliff failures when an app uses
	 * its quota at the beginning of the window.
	 */
	refill?: {
		tokens: number;
		windowSecs: number;
	};
	/**
	 * Auto-expire keys after this many days. `0` = never (the
	 * default). Apps in regulated industries (SOC2-strict) usually
	 * set 90.
	 */
	expiresInDays?: number;
	/**
	 * Permission catalog. Map of permission name → array of role-
	 * style scopes. The plugin doesn't enforce these — that's the
	 * app's job — but it persists them on the key + exposes them
	 * via `parseAndAuthorize` so the app can check.
	 */
	knownPermissions?: string[];
}

export interface CreateKeyInput {
	name: string;
	userId?: string;
	orgId?: string;
	permissions?: string[];
	expiresAt?: string;
	rateLimitMax?: number;
	rateLimitWindowSecs?: number;
}

export interface MintedKey {
	id: string;
	/** RAW key — show to user exactly once, never persist. */
	rawKey: string;
	prefix: string;
	createdAt: string;
}

export async function mintKey(
	cfg: ApiKeyConfig,
	input: CreateKeyInput,
): Promise<{
	row: {
		name: string;
		prefix: string;
		hash: string;
		userId: string | null;
		orgId: string | null;
		permissions: string;
		rateLimitMax: number | null;
		rateLimitWindowSecs: number | null;
		refillTokens: number | null;
		refillUpdatedAt: string | null;
		expiresAt: string | null;
		createdAt: string;
	};
	rawKey: string;
}> {
	const env = cfg.env ?? "live";
	const raw = mintRawKey(env);
	const hash = await hashKey(raw);
	const prefix = raw.slice(0, 12); // "pk_live_XXXX" — first 4 random chars
	const now = new Date().toISOString();
	const expiresAt =
		input.expiresAt ??
		(cfg.expiresInDays && cfg.expiresInDays > 0
			? new Date(Date.now() + cfg.expiresInDays * 86_400_000).toISOString()
			: null);
	const rateLimitMax =
		input.rateLimitMax ?? cfg.defaultRateLimit?.max ?? null;
	const rateLimitWindowSecs =
		input.rateLimitWindowSecs ?? cfg.defaultRateLimit?.windowSecs ?? null;
	return {
		row: {
			name: input.name,
			prefix,
			hash,
			userId: input.userId ?? null,
			orgId: input.orgId ?? null,
			permissions: JSON.stringify(input.permissions ?? []),
			rateLimitMax,
			rateLimitWindowSecs,
			refillTokens: rateLimitMax ?? null,
			refillUpdatedAt: now,
			expiresAt,
			createdAt: now,
		},
		rawKey: raw,
	};
}

export interface AuthorizedKey {
	id: string;
	userId?: string | null;
	orgId?: string | null;
	permissions: string[];
	rateLimit: RateLimitState | null;
}

/**
 * Given a raw key + a row loader, return the authorized key state
 * (including post-decrement rate-limit bucket) or null on mismatch.
 *
 * The row loader hides the storage details — apps wire it to their
 * `ctx.db.query("ApiKey", { prefix })` lookup. The plugin only sees
 * hashed rows, never plaintext keys.
 */
export async function parseAndAuthorize(
	rawKey: string,
	loadByPrefix: (
		prefix: string,
	) => Promise<Array<{
		id: string;
		hash: string;
		userId?: string | null;
		orgId?: string | null;
		permissions: string;
		expiresAt?: string | null;
		rateLimitMax?: number | null;
		rateLimitWindowSecs?: number | null;
		refillTokens?: number | null;
		refillUpdatedAt?: string | null;
	}> | undefined>,
	persistBucket?: (
		id: string,
		bucket: { refillTokens: number; refillUpdatedAt: string },
	) => Promise<void>,
	now: number = Date.now(),
): Promise<AuthorizedKey | null> {
	const parsed = parseKey(rawKey);
	if (!parsed) return null;
	const rows = (await loadByPrefix(rawKey.slice(0, 12))) ?? [];
	for (const row of rows) {
		if (row.expiresAt && new Date(row.expiresAt).getTime() < now) continue;
		const actualHash = await hashKey(rawKey);
		if (!constantTimeEqual(actualHash, row.hash)) continue;

		// Rate-limit check + refill.
		let limit: RateLimitState | null = null;
		if (row.rateLimitMax && row.rateLimitWindowSecs) {
			const decision = checkAndConsume({
				max: row.rateLimitMax,
				windowSecs: row.rateLimitWindowSecs,
				tokens: row.refillTokens ?? row.rateLimitMax,
				updatedAt: row.refillUpdatedAt
					? new Date(row.refillUpdatedAt).getTime()
					: now,
				now,
			});
			if (!decision.allowed) return null;
			limit = {
				tokens: decision.tokens,
				updatedAt: new Date(decision.updatedAt).toISOString(),
				max: row.rateLimitMax,
				windowSecs: row.rateLimitWindowSecs,
			};
			if (persistBucket) {
				await persistBucket(row.id, {
					refillTokens: decision.tokens,
					refillUpdatedAt: new Date(decision.updatedAt).toISOString(),
				});
			}
		}

		return {
			id: row.id,
			userId: row.userId,
			orgId: row.orgId,
			permissions: safeParseArray(row.permissions),
			rateLimit: limit,
		};
	}
	return null;
}

function constantTimeEqual(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	return diff === 0;
}

function safeParseArray(s: string): string[] {
	try {
		const out = JSON.parse(s);
		return Array.isArray(out) ? (out as string[]) : [];
	} catch {
		return [];
	}
}
