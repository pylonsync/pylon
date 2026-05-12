/**
 * Key generation + hashing.
 *
 * Format: `pk_<env>_<base32>` where the random body is 32 chars of
 * base32 (no padding, Crockford-style). Yields ~160 bits of entropy
 * — more than enough to defeat brute-force given hashed storage +
 * rate limiting on the verify path.
 *
 * SHA-256 hashing with a per-process random salt. We could use
 * Argon2 like the auth crate does for passwords; we don't because
 * API keys have full entropy from generation (not user-chosen) so
 * the password-hash compute cost is unnecessary on every request.
 */

const BASE32 = "ABCDEFGHIJKLMNPQRSTUVWXYZ23456789"; // no O or 0

export type KeyEnv = "live" | "test";

export function mintRawKey(env: KeyEnv): string {
	const bytes = new Uint8Array(20); // 20 bytes → 32 base32 chars
	crypto.getRandomValues(bytes);
	let body = "";
	for (const b of bytes) body += BASE32[b % BASE32.length];
	return `pk_${env}_${body}`;
}

export function parseKey(raw: string): { env: KeyEnv; body: string } | null {
	if (!raw.startsWith("pk_")) return null;
	const parts = raw.split("_");
	if (parts.length !== 3) return null;
	const env = parts[1];
	if (env !== "live" && env !== "test") return null;
	if (parts[2].length < 16) return null; // sanity
	return { env, body: parts[2] };
}

/**
 * Hash a key for storage. Salt is bundled into the output so a
 * single DB column suffices. Format: `<salt-hex>:<sha256-hex>`.
 */
export async function hashKey(raw: string): Promise<string> {
	const salt = new Uint8Array(16);
	crypto.getRandomValues(salt);
	const data = new TextEncoder().encode(raw);
	const combined = new Uint8Array(salt.length + data.length);
	combined.set(salt, 0);
	combined.set(data, salt.length);
	const buf = new ArrayBuffer(combined.byteLength);
	new Uint8Array(buf).set(combined);
	const hash = await crypto.subtle.digest("SHA-256", buf);
	return `${hex(salt)}:${hex(new Uint8Array(hash))}`;
}

/**
 * Re-hash a key with a pre-existing salt. Used internally to
 * compare a candidate key against a stored hash without
 * round-tripping through the random-salt path.
 */
export async function rehash(raw: string, stored: string): Promise<boolean> {
	const parts = stored.split(":");
	if (parts.length !== 2) return false;
	const salt = unhex(parts[0]);
	const expected = parts[1];
	const data = new TextEncoder().encode(raw);
	const combined = new Uint8Array(salt.length + data.length);
	combined.set(salt, 0);
	combined.set(data, salt.length);
	const buf = new ArrayBuffer(combined.byteLength);
	new Uint8Array(buf).set(combined);
	const hash = await crypto.subtle.digest("SHA-256", buf);
	const actual = hex(new Uint8Array(hash));
	if (actual.length !== expected.length) return false;
	let diff = 0;
	for (let i = 0; i < actual.length; i++) {
		diff |= actual.charCodeAt(i) ^ expected.charCodeAt(i);
	}
	return diff === 0;
}

function hex(b: Uint8Array): string {
	return [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
}
function unhex(s: string): Uint8Array {
	const out = new Uint8Array(s.length / 2);
	for (let i = 0; i < out.length; i++) {
		out[i] = Number.parseInt(s.slice(i * 2, i * 2 + 2), 16);
	}
	return out;
}
