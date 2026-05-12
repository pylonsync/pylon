import { describe, expect, test } from "bun:test";

import { hashKey, mintRawKey, parseKey, rehash } from "./keys";
import { checkAndConsume } from "./rate-limit";

describe("key generation", () => {
	test("mintRawKey yields parseable env-tagged keys", () => {
		const liveKey = mintRawKey("live");
		const testKey = mintRawKey("test");
		expect(liveKey.startsWith("pk_live_")).toBe(true);
		expect(testKey.startsWith("pk_test_")).toBe(true);

		const parsedLive = parseKey(liveKey);
		const parsedTest = parseKey(testKey);
		expect(parsedLive?.env).toBe("live");
		expect(parsedTest?.env).toBe("test");
	});

	test("parseKey rejects malformed inputs", () => {
		expect(parseKey("not-a-key")).toBeNull();
		expect(parseKey("pk_bad_xxx")).toBeNull();
		expect(parseKey("pk_live_short")).toBeNull();
	});

	test("hashKey + rehash round-trip", async () => {
		const raw = mintRawKey("live");
		const stored = await hashKey(raw);
		expect(await rehash(raw, stored)).toBe(true);
		expect(await rehash(mintRawKey("live"), stored)).toBe(false);
	});
});

describe("rate limit (token bucket)", () => {
	test("allows when bucket has capacity", () => {
		const dec = checkAndConsume({
			max: 100,
			windowSecs: 60,
			tokens: 50,
			updatedAt: 1_000_000,
			now: 1_000_000,
		});
		expect(dec.allowed).toBe(true);
		expect(dec.tokens).toBeCloseTo(49, 1);
	});

	test("denies when bucket is empty + no time elapsed", () => {
		const dec = checkAndConsume({
			max: 100,
			windowSecs: 60,
			tokens: 0,
			updatedAt: 1_000_000,
			now: 1_000_000,
		});
		expect(dec.allowed).toBe(false);
	});

	test("refills linearly with elapsed time", () => {
		// max 60 / 60s = 1 token/sec refill rate.
		const dec = checkAndConsume({
			max: 60,
			windowSecs: 60,
			tokens: 0,
			updatedAt: 1_000_000,
			now: 1_000_000 + 10_000, // 10s later
		});
		// 10 tokens refilled, consume one, so 9 remain.
		expect(dec.allowed).toBe(true);
		expect(dec.tokens).toBeCloseTo(9, 1);
	});

	test("refill caps at max (no over-fill)", () => {
		const dec = checkAndConsume({
			max: 60,
			windowSecs: 60,
			tokens: 50,
			updatedAt: 1_000_000,
			now: 1_000_000 + 60_000 * 60, // 60 minutes later — way past
		});
		expect(dec.allowed).toBe(true);
		// Pre-consumption was capped at 60 — post-consumption is 59.
		expect(dec.tokens).toBeCloseTo(59, 1);
	});
});
