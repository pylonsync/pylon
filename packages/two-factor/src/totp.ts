/**
 * RFC 6238 TOTP. SHA-1, 6 digits, 30-second window by default —
 * matches what every authenticator app (Google, Authy, 1Password)
 * generates.
 *
 * Pure WebCrypto + base32. No third-party dependency.
 */

const BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

export interface TotpConfig {
	digits?: number;
	period?: number;
	algorithm?: "SHA-1" | "SHA-256" | "SHA-512";
}

export function generateSecret(byteLength = 20): string {
	const bytes = new Uint8Array(byteLength);
	crypto.getRandomValues(bytes);
	return base32Encode(bytes);
}

export function otpAuthUri(opts: {
	issuer: string;
	accountName: string;
	secret: string;
	digits?: number;
	period?: number;
	algorithm?: string;
}): string {
	const params = new URLSearchParams({
		secret: opts.secret,
		issuer: opts.issuer,
		algorithm: opts.algorithm ?? "SHA1",
		digits: String(opts.digits ?? 6),
		period: String(opts.period ?? 30),
	});
	const label = `${encodeURIComponent(opts.issuer)}:${encodeURIComponent(opts.accountName)}`;
	return `otpauth://totp/${label}?${params.toString()}`;
}

export async function generateTotp(
	secret: string,
	cfg: TotpConfig = {},
	now: number = Date.now(),
): Promise<string> {
	const digits = cfg.digits ?? 6;
	const period = cfg.period ?? 30;
	const algorithm = cfg.algorithm ?? "SHA-1";
	const counter = Math.floor(now / 1000 / period);
	return hotp(secret, counter, digits, algorithm);
}

/**
 * Verify a TOTP code with skew tolerance + replay protection.
 *
 * `lastAcceptedCounter` is the time-step counter of the most-recent
 * accepted code for this user; the caller persists it (e.g. on the
 * `twoFactor.lastAcceptedCounter` column) and passes it in so a
 * second submission of the same 30-second window can't replay. RFC
 * 6238 §5.2 explicitly calls this out — without it, a code observed
 * by an attacker (shoulder-surfing, log leak) was usable for up to
 * 30 seconds.
 *
 * Returns the counter of the accepted code (which the caller should
 * persist as the new `lastAcceptedCounter`), or `null` on mismatch.
 */
export async function verifyTotp(
	secret: string,
	code: string,
	cfg: TotpConfig & {
		windowSteps?: number;
		lastAcceptedCounter?: number;
	} = {},
	now: number = Date.now(),
): Promise<number | null> {
	const digits = cfg.digits ?? 6;
	const period = cfg.period ?? 30;
	const algorithm = cfg.algorithm ?? "SHA-1";
	const window = cfg.windowSteps ?? 1;
	const counter = Math.floor(now / 1000 / period);
	for (let offset = -window; offset <= window; offset++) {
		const c = counter + offset;
		if (cfg.lastAcceptedCounter !== undefined && c <= cfg.lastAcceptedCounter) {
			// Already-used window — replay protection.
			continue;
		}
		const expected = await hotp(secret, c, digits, algorithm);
		if (constantTimeEqual(expected, code)) return c;
	}
	return null;
}

async function hotp(
	secret: string,
	counter: number,
	digits: number,
	algorithm: "SHA-1" | "SHA-256" | "SHA-512",
): Promise<string> {
	const keyBytes = base32Decode(secret);
	const counterBytes = new Uint8Array(8);
	const view = new DataView(counterBytes.buffer);
	view.setBigUint64(0, BigInt(counter), false);

	const key = await crypto.subtle.importKey(
		"raw",
		toArrayBuffer(keyBytes),
		{ name: "HMAC", hash: algorithm },
		false,
		["sign"],
	);
	const sig = new Uint8Array(
		await crypto.subtle.sign("HMAC", key, toArrayBuffer(counterBytes)),
	);
	// Dynamic truncation (RFC 4226 §5.3).
	const offset = sig[sig.length - 1] & 0x0f;
	const code =
		((sig[offset] & 0x7f) << 24) |
		((sig[offset + 1] & 0xff) << 16) |
		((sig[offset + 2] & 0xff) << 8) |
		(sig[offset + 3] & 0xff);
	const mod = 10 ** digits;
	return (code % mod).toString().padStart(digits, "0");
}

function constantTimeEqual(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	return diff === 0;
}

function base32Encode(bytes: Uint8Array): string {
	let bits = 0;
	let value = 0;
	let out = "";
	for (const b of bytes) {
		value = (value << 8) | b;
		bits += 8;
		while (bits >= 5) {
			out += BASE32_ALPHABET[(value >>> (bits - 5)) & 0x1f];
			bits -= 5;
		}
	}
	if (bits > 0) out += BASE32_ALPHABET[(value << (5 - bits)) & 0x1f];
	return out;
}

function base32Decode(s: string): Uint8Array {
	const clean = s.replace(/=+$/, "").toUpperCase().replace(/\s/g, "");
	let bits = 0;
	let value = 0;
	const out: number[] = [];
	for (const ch of clean) {
		const idx = BASE32_ALPHABET.indexOf(ch);
		if (idx === -1) continue;
		value = (value << 5) | idx;
		bits += 5;
		if (bits >= 8) {
			out.push((value >>> (bits - 8)) & 0xff);
			bits -= 8;
		}
	}
	return new Uint8Array(out);
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
	const buf = new ArrayBuffer(bytes.byteLength);
	new Uint8Array(buf).set(bytes);
	return buf;
}
