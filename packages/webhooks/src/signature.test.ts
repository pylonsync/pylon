import { describe, expect, test } from "bun:test";

import { signWebhook, verifyWebhook } from "./signature";

const SECRET = "whsec_dGVzdC1zZWNyZXQtdmFsdWU="; // base64('test-secret-value')
const ID = "evt_test_123";
const BODY = '{"id":"evt_test_123","type":"x.y","data":{}}';

describe("webhook signature", () => {
	test("sign + verify round-trip succeeds", async () => {
		const ts = 1_700_000_000;
		const sig = await signWebhook({
			id: ID,
			timestamp: ts,
			body: BODY,
			secrets: [SECRET],
		});
		const result = await verifyWebhook(
			SECRET,
			{ id: sig.id, timestamp: String(sig.timestamp), signature: sig.signature },
			BODY,
			{ nowSecs: ts },
		);
		expect(result).toBe(true);
	});

	test("verify rejects a stale timestamp (replay window)", async () => {
		const ts = 1_700_000_000;
		const sig = await signWebhook({
			id: ID,
			timestamp: ts,
			body: BODY,
			secrets: [SECRET],
		});
		const result = await verifyWebhook(
			SECRET,
			{ id: sig.id, timestamp: String(sig.timestamp), signature: sig.signature },
			BODY,
			{ nowSecs: ts + 10 * 60 }, // 10 min later
		);
		expect(result).toBe("REPLAYED");
	});

	test("verify rejects a tampered body", async () => {
		const ts = 1_700_000_000;
		const sig = await signWebhook({
			id: ID,
			timestamp: ts,
			body: BODY,
			secrets: [SECRET],
		});
		const result = await verifyWebhook(
			SECRET,
			{ id: sig.id, timestamp: String(sig.timestamp), signature: sig.signature },
			`${BODY}x`, // tampered
			{ nowSecs: ts },
		);
		expect(result).toBe("INVALID_SIGNATURE");
	});

	test("verify accepts either signature during rotation", async () => {
		const ts = 1_700_000_000;
		const oldSecret = "whsec_b2xkLXNlY3JldA=="; // base64('old-secret')
		const sig = await signWebhook({
			id: ID,
			timestamp: ts,
			body: BODY,
			secrets: [oldSecret, SECRET], // old + new
		});
		// Receiver using only the new secret still accepts.
		expect(
			await verifyWebhook(
				SECRET,
				{ id: sig.id, timestamp: String(sig.timestamp), signature: sig.signature },
				BODY,
				{ nowSecs: ts },
			),
		).toBe(true);
		// Receiver still on the old secret also accepts.
		expect(
			await verifyWebhook(
				oldSecret,
				{ id: sig.id, timestamp: String(sig.timestamp), signature: sig.signature },
				BODY,
				{ nowSecs: ts },
			),
		).toBe(true);
	});

	test("verify reports missing headers", async () => {
		expect(
			await verifyWebhook(SECRET, { id: null, timestamp: "1", signature: "v1,x" }, BODY),
		).toBe("MISSING_ID");
		expect(
			await verifyWebhook(SECRET, { id: "x", timestamp: null, signature: "v1,x" }, BODY),
		).toBe("MISSING_TIMESTAMP");
		expect(
			await verifyWebhook(SECRET, { id: "x", timestamp: "1", signature: null }, BODY),
		).toBe("MISSING_SIGNATURE");
	});
});
