/**
 * Svix-compatible HMAC-SHA256 signing for outbound webhooks.
 *
 * Headers we attach:
 *   - `webhook-id`: stable event id (used by receivers for dedup)
 *   - `webhook-timestamp`: unix seconds
 *   - `webhook-signature`: `v1,<base64-sig> [v1,<base64-sig-2>...]`
 *     Multiple v1s appear during secret rotation — both old and new
 *     secrets are signed simultaneously for a configurable overlap
 *     window so receivers can update their secret without dropping
 *     deliveries.
 *
 * Signature input: `<webhook-id>.<webhook-timestamp>.<body>`
 *
 * Conformant with the Svix signature spec so existing receivers
 * (Resend, Clerk, anyone using their reference verifier) work
 * unchanged.
 */

export interface SignOptions {
	id: string;
	timestamp: number;
	body: string;
	secrets: string[];
}

export interface WebhookSignaturePayload {
	id: string;
	timestamp: number;
	signature: string;
}

export async function signWebhook(
	opts: SignOptions,
): Promise<WebhookSignaturePayload> {
	const sigs: string[] = [];
	for (const secret of opts.secrets) {
		const sig = await hmacSha256B64(
			normalizeSecret(secret),
			`${opts.id}.${opts.timestamp}.${opts.body}`,
		);
		sigs.push(`v1,${sig}`);
	}
	return {
		id: opts.id,
		timestamp: opts.timestamp,
		signature: sigs.join(" "),
	};
}

export type SignatureError =
	| "MISSING_ID"
	| "MISSING_TIMESTAMP"
	| "MISSING_SIGNATURE"
	| "REPLAYED"
	| "INVALID_SIGNATURE";

export async function verifyWebhook(
	secret: string,
	headers: {
		id?: string | null;
		timestamp?: string | null;
		signature?: string | null;
	},
	body: string,
	opts: { toleranceSecs?: number; nowSecs?: number } = {},
): Promise<true | SignatureError> {
	const id = headers.id;
	const ts = headers.timestamp;
	const sig = headers.signature;
	if (!id) return "MISSING_ID";
	if (!ts) return "MISSING_TIMESTAMP";
	if (!sig) return "MISSING_SIGNATURE";

	const tsNum = Number.parseInt(ts, 10);
	if (Number.isNaN(tsNum)) return "MISSING_TIMESTAMP";
	const now = opts.nowSecs ?? Math.floor(Date.now() / 1000);
	const tolerance = opts.toleranceSecs ?? 300;
	if (Math.abs(now - tsNum) > tolerance) return "REPLAYED";

	const expected = await hmacSha256B64(
		normalizeSecret(secret),
		`${id}.${ts}.${body}`,
	);
	for (const part of sig.split(" ")) {
		const [version, candidate] = part.split(",");
		if (version !== "v1") continue;
		if (constantTimeEqual(candidate, expected)) return true;
	}
	return "INVALID_SIGNATURE";
}

function normalizeSecret(secret: string): Uint8Array {
	if (secret.startsWith("whsec_")) {
		return base64Decode(secret.slice("whsec_".length));
	}
	return new TextEncoder().encode(secret);
}

async function hmacSha256B64(
	secret: Uint8Array,
	payload: string,
): Promise<string> {
	const secretBuf = new ArrayBuffer(secret.byteLength);
	new Uint8Array(secretBuf).set(secret);
	const bodyBytes = new TextEncoder().encode(payload);
	const bodyBuf = new ArrayBuffer(bodyBytes.byteLength);
	new Uint8Array(bodyBuf).set(bodyBytes);
	const key = await crypto.subtle.importKey(
		"raw",
		secretBuf,
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	const sig = await crypto.subtle.sign("HMAC", key, bodyBuf);
	return base64Encode(new Uint8Array(sig));
}

function base64Decode(s: string): Uint8Array {
	const bin = atob(s);
	const out = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
	return out;
}
function base64Encode(bytes: Uint8Array): string {
	let s = "";
	for (const b of bytes) s += String.fromCharCode(b);
	return btoa(s);
}
function constantTimeEqual(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	return diff === 0;
}
