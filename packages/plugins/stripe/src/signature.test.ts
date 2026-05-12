import { expect, test } from "bun:test";

import { verifyStripeSignature } from "./signature";

const SECRET = "whsec_test_secret_value";
const RAW_BODY = '{"id":"evt_test","type":"customer.subscription.created"}';

async function hmacHex(secret: string, payload: string): Promise<string> {
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

async function signedHeader(
	secret: string,
	rawBody: string,
	tsSecs: number,
): Promise<string> {
	const sig = await hmacHex(secret, `${tsSecs}.${rawBody}`);
	return `t=${tsSecs},v1=${sig}`;
}

test("verify accepts a valid signature within tolerance", async () => {
	const now = 1_700_000_000;
	const header = await signedHeader(SECRET, RAW_BODY, now);
	const result = await verifyStripeSignature(SECRET, RAW_BODY, header, {
		nowSecs: now,
	});
	expect(result).toBe(true);
});

test("verify rejects a stale timestamp (replay window)", async () => {
	const eventTs = 1_700_000_000;
	const now = eventTs + 10 * 60; // 10 min later
	const header = await signedHeader(SECRET, RAW_BODY, eventTs);
	const result = await verifyStripeSignature(SECRET, RAW_BODY, header, {
		nowSecs: now,
	});
	expect(result).toBe("REPLAYED");
});

test("verify rejects a tampered body", async () => {
	const now = 1_700_000_000;
	const header = await signedHeader(SECRET, RAW_BODY, now);
	const result = await verifyStripeSignature(
		SECRET,
		`${RAW_BODY}x`, // body changed after signing
		header,
		{ nowSecs: now },
	);
	expect(result).toBe("INVALID_SIGNATURE");
});

test("verify rejects a wrong secret", async () => {
	const now = 1_700_000_000;
	const header = await signedHeader(SECRET, RAW_BODY, now);
	const result = await verifyStripeSignature(
		"whsec_different",
		RAW_BODY,
		header,
		{ nowSecs: now },
	);
	expect(result).toBe("INVALID_SIGNATURE");
});

test("verify reports MISSING_HEADER on null/undefined", async () => {
	expect(await verifyStripeSignature(SECRET, RAW_BODY, null)).toBe(
		"MISSING_HEADER",
	);
	expect(await verifyStripeSignature(SECRET, RAW_BODY, undefined)).toBe(
		"MISSING_HEADER",
	);
});

test("verify accepts either of multiple v1 signatures (secret rotation)", async () => {
	const now = 1_700_000_000;
	const sigOld = await hmacHex("whsec_old", `${now}.${RAW_BODY}`);
	const sigNew = await hmacHex(SECRET, `${now}.${RAW_BODY}`);
	const header = `t=${now},v1=${sigOld},v1=${sigNew}`;
	const result = await verifyStripeSignature(SECRET, RAW_BODY, header, {
		nowSecs: now,
	});
	expect(result).toBe(true);
});
