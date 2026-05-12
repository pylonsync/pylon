import { describe, expect, test } from "bun:test";

import {
	generateBackupCodes,
	hashBackupCode,
	verifyBackupCode,
} from "./backup-codes";

describe("backup codes", () => {
	test("generated codes match the expected format", () => {
		const codes = generateBackupCodes();
		expect(codes).toHaveLength(10);
		for (const c of codes) {
			expect(c).toMatch(/^[A-Z2-9]{4}-[A-Z2-9]{4}$/);
		}
	});

	test("hash → verify round-trip succeeds", async () => {
		const codes = generateBackupCodes({ amount: 2 });
		const hashes = await Promise.all(codes.map(hashBackupCode));
		expect(await verifyBackupCode(codes[0], hashes[0])).toBe(true);
		expect(await verifyBackupCode(codes[1], hashes[1])).toBe(true);
	});

	test("hash → verify with wrong code fails", async () => {
		const [code] = generateBackupCodes({ amount: 1 });
		const hash = await hashBackupCode(code);
		expect(await verifyBackupCode("WRONG-CODE", hash)).toBe(false);
	});

	test("verify gracefully fails on malformed stored value", async () => {
		expect(await verifyBackupCode("ANY-CODE", "")).toBe(false);
		expect(await verifyBackupCode("ANY-CODE", "garbage")).toBe(false);
	});

	test("codes do NOT include confusable characters (0, O, 1, I)", () => {
		const codes = generateBackupCodes({ amount: 100 });
		const joined = codes.join("");
		expect(joined).not.toMatch(/[0O1I]/);
	});
});
