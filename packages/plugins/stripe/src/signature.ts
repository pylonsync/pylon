/**
 * Stripe webhook signature verification.
 *
 * Stripe signs every webhook payload with HMAC-SHA256 over
 * `<timestamp>.<rawBody>` and includes the signature + timestamp in
 * the `Stripe-Signature` header (format: `t=<unix>,v1=<sig>,...`).
 *
 * Spec: https://stripe.com/docs/webhooks/signatures
 *
 * Constants:
 *   - 5-minute replay window (matches Stripe's CLI tool default).
 *   - Constant-time signature compare to defeat timing oracles.
 *
 * Implementation uses the WebCrypto API (Bun + Node 20+) — no
 * third-party crypto dep.
 */

const DEFAULT_TOLERANCE_SECS = 300;

export type SignatureError =
	| "MISSING_HEADER"
	| "MISSING_TIMESTAMP"
	| "MISSING_SIGNATURE"
	| "REPLAYED"
	| "INVALID_SIGNATURE";

/**
 * Returns `true` when the signature header matches an HMAC of the
 * raw body under the given secret AND the timestamp is within the
 * replay window. Returns a string error code otherwise.
 *
 * `nowSecs` lets tests inject a fixed clock; production should leave
 * it undefined to use `Date.now()`.
 */
export async function verifyStripeSignature(
	secret: string,
	rawBody: string,
	headerValue: string | undefined | null,
	opts: { toleranceSecs?: number; nowSecs?: number } = {},
): Promise<true | SignatureError> {
	if (!headerValue) return "MISSING_HEADER";
	const tolerance = opts.toleranceSecs ?? DEFAULT_TOLERANCE_SECS;
	const now = opts.nowSecs ?? Math.floor(Date.now() / 1000);

	// Header shape: `t=<unix>,v1=<sig>[,v1=<sig2>...]`. Multiple v1s
	// appear during signing-secret rotation — accept either.
	let timestamp: number | null = null;
	const signatures: string[] = [];
	for (const part of headerValue.split(",")) {
		const [k, v] = part.split("=");
		if (!k || !v) continue;
		if (k.trim() === "t") timestamp = Number.parseInt(v.trim(), 10);
		else if (k.trim() === "v1") signatures.push(v.trim());
	}
	if (timestamp === null || Number.isNaN(timestamp)) return "MISSING_TIMESTAMP";
	if (signatures.length === 0) return "MISSING_SIGNATURE";
	if (Math.abs(now - timestamp) > tolerance) return "REPLAYED";

	const expected = await hmacSha256Hex(secret, `${timestamp}.${rawBody}`);
	for (const sig of signatures) {
		if (constantTimeEqualHex(sig, expected)) return true;
	}
	return "INVALID_SIGNATURE";
}

async function hmacSha256Hex(secret: string, payload: string): Promise<string> {
	const enc = new TextEncoder();
	const key = await crypto.subtle.importKey(
		"raw",
		enc.encode(secret),
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	const sig = await crypto.subtle.sign("HMAC", key, enc.encode(payload));
	return [...new Uint8Array(sig)]
		.map((b) => b.toString(16).padStart(2, "0"))
		.join("");
}

function constantTimeEqualHex(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) {
		diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	}
	return diff === 0;
}
