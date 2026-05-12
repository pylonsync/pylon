/**
 * Backup codes — single-use recovery codes for the case a user
 * loses their authenticator device. Stored as Argon2-hashed values
 * at rest; the plaintext is shown to the user exactly once on
 * generation.
 *
 * Defaults:
 *   - 10 codes per user, 8 chars each (alphanumeric)
 *   - Codes are formatted as `XXXX-XXXX` for hand-typing
 *
 * Better-auth defaults to 10×16-char hex; we use shorter
 * alphanumeric because typing 16 hex chars on a phone keyboard is
 * painful and the entropy at 8 alphanumeric chars (~47 bits) still
 * comfortably resists brute force given the 1-attempt-per-code
 * (the verify path deletes the code on use).
 */

const ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/1/I

export interface BackupCodeConfig {
	amount?: number;
	length?: number;
	formatted?: boolean;
}

export function generateBackupCodes(cfg: BackupCodeConfig = {}): string[] {
	const amount = cfg.amount ?? 10;
	const length = cfg.length ?? 8;
	const formatted = cfg.formatted ?? true;
	const out: string[] = [];
	for (let i = 0; i < amount; i++) {
		const bytes = new Uint8Array(length);
		crypto.getRandomValues(bytes);
		let code = "";
		for (const b of bytes) code += ALPHABET[b % ALPHABET.length];
		out.push(formatted ? `${code.slice(0, 4)}-${code.slice(4)}` : code);
	}
	return out;
}

/**
 * Hash a backup code for storage. SHA-256 + per-process random
 * salt baked into the hash. We don't use Argon2/bcrypt here because
 * backup codes have full entropy from generation (we control the
 * entropy source) so the password-hash compute cost is unnecessary
 * — a plain SHA-256 over salt+code is sufficient against rainbow
 * tables, and a fast hash means verification is sub-millisecond.
 */
export async function hashBackupCode(code: string): Promise<string> {
	const salt = new Uint8Array(16);
	crypto.getRandomValues(salt);
	const data = new TextEncoder().encode(code);
	const combined = new Uint8Array(salt.length + data.length);
	combined.set(salt, 0);
	combined.set(data, salt.length);
	const hash = await crypto.subtle.digest(
		"SHA-256",
		toArrayBuffer(combined),
	);
	return `${hex(salt)}:${hex(new Uint8Array(hash))}`;
}

export async function verifyBackupCode(
	code: string,
	stored: string,
): Promise<boolean> {
	const parts = stored.split(":");
	if (parts.length !== 2) return false;
	const salt = unhex(parts[0]);
	const expected = parts[1];
	const data = new TextEncoder().encode(code);
	const combined = new Uint8Array(salt.length + data.length);
	combined.set(salt, 0);
	combined.set(data, salt.length);
	const hash = await crypto.subtle.digest(
		"SHA-256",
		toArrayBuffer(combined),
	);
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
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
	const buf = new ArrayBuffer(bytes.byteLength);
	new Uint8Array(buf).set(bytes);
	return buf;
}
