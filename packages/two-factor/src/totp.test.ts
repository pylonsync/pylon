import { describe, expect, test } from "bun:test";

import {
	generateSecret,
	generateTotp,
	otpAuthUri,
	verifyTotp,
} from "./totp";

describe("TOTP", () => {
	test("generated secret round-trips through verify", async () => {
		const secret = generateSecret();
		const now = 1_700_000_000_000;
		const code = await generateTotp(secret, {}, now);
		expect(code).toHaveLength(6);
		const counter = await verifyTotp(secret, code, {}, now);
		expect(counter).not.toBeNull();
	});

	test("verify rejects a code from a different secret", async () => {
		const s1 = generateSecret();
		const s2 = generateSecret();
		const now = 1_700_000_000_000;
		const code = await generateTotp(s1, {}, now);
		const counter = await verifyTotp(s2, code, {}, now);
		expect(counter).toBeNull();
	});

	test("verify rejects a stale code (outside window)", async () => {
		const secret = generateSecret();
		const now = 1_700_000_000_000;
		const code = await generateTotp(secret, {}, now);
		// 2 minutes later — outside the default ±1 window (30s steps).
		const later = now + 2 * 60 * 1000;
		const counter = await verifyTotp(secret, code, {}, later);
		expect(counter).toBeNull();
	});

	test("verify accepts a code at the edge of the skew window", async () => {
		const secret = generateSecret();
		const now = 1_700_000_000_000;
		const code = await generateTotp(secret, {}, now);
		// 30 seconds later — within the ±1 step window.
		const later = now + 30 * 1000;
		const counter = await verifyTotp(secret, code, {}, later);
		expect(counter).not.toBeNull();
	});

	test("verify enforces replay protection via lastAcceptedCounter", async () => {
		const secret = generateSecret();
		const now = 1_700_000_000_000;
		const code = await generateTotp(secret, {}, now);
		const first = await verifyTotp(secret, code, {}, now);
		expect(first).not.toBeNull();

		// Pretend we persisted the counter — re-submitting the same
		// code in the same window must fail.
		const replay = await verifyTotp(
			secret,
			code,
			{ lastAcceptedCounter: first ?? 0 },
			now,
		);
		expect(replay).toBeNull();
	});

	test("otpAuthUri produces a scannable otpauth:// URI", () => {
		const uri = otpAuthUri({
			issuer: "Acme",
			accountName: "user@example.com",
			secret: "JBSWY3DPEHPK3PXP",
		});
		expect(uri).toContain("otpauth://totp/");
		expect(uri).toContain("issuer=Acme");
		expect(uri).toContain("digits=6");
		expect(uri).toContain("period=30");
	});
});
