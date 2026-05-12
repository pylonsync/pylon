import { describe, expect, test } from "bun:test";

import { evaluate, hashBucket } from "./eval";
import type { BooleanFlag, MultivariateFlag } from "./types";

describe("hashBucket", () => {
	test("0% rollout always returns false", () => {
		expect(hashBucket("user_1", 0)).toBe(false);
		expect(hashBucket("user_2", 0)).toBe(false);
	});

	test("100% rollout always returns true", () => {
		expect(hashBucket("user_1", 100)).toBe(true);
		expect(hashBucket("user_2", 100)).toBe(true);
	});

	test("50% rollout distributes ~uniformly", () => {
		let hits = 0;
		const N = 10000;
		for (let i = 0; i < N; i++) {
			if (hashBucket(`u_${i}`, 50)) hits++;
		}
		const ratio = hits / N;
		expect(ratio).toBeGreaterThan(0.45);
		expect(ratio).toBeLessThan(0.55);
	});

	test("same key + percent = same answer (deterministic)", () => {
		const v1 = hashBucket("user_42", 25);
		const v2 = hashBucket("user_42", 25);
		expect(v1).toBe(v2);
	});
});

describe("boolean flag evaluation", () => {
	test("default value when no rollout", () => {
		const flag: BooleanFlag = { type: "boolean", default: true };
		const r = evaluate(flag, { userId: "u_1" });
		expect(r.value).toBe(true);
		expect(r.matched).toBe("default");
	});

	test("rollout splits users", () => {
		const flag: BooleanFlag = {
			type: "boolean",
			default: true,
			rollout: { percent: 100 },
		};
		const r = evaluate(flag, { userId: "u_1" });
		expect(r.value).toBe(true);
		expect(r.matched).toBe("rollout");
	});

	test("targeting rule beats rollout", () => {
		const flag: BooleanFlag = {
			type: "boolean",
			default: false,
			rollout: { percent: 0 },
			targeting: [
				{
					value: true,
					when: [{ property: "plan", op: "eq", value: "enterprise" }],
				},
			],
		};
		const r = evaluate(flag, {
			userId: "u_1",
			properties: { plan: "enterprise" },
		});
		expect(r.value).toBe(true);
		expect(r.matched).toBe("targeting");
	});

	test("targeting predicates AND together", () => {
		const flag: BooleanFlag = {
			type: "boolean",
			default: false,
			targeting: [
				{
					value: true,
					when: [
						{ property: "plan", op: "eq", value: "pro" },
						{ property: "betaTester", op: "eq", value: true },
					],
				},
			],
		};
		// Only one predicate satisfied — should fall back to default.
		const r1 = evaluate(flag, {
			userId: "u_1",
			properties: { plan: "pro", betaTester: false },
		});
		expect(r1.value).toBe(false);
		// Both satisfied — rule fires.
		const r2 = evaluate(flag, {
			userId: "u_1",
			properties: { plan: "pro", betaTester: true },
		});
		expect(r2.value).toBe(true);
	});

	test("contains / starts_with / ends_with predicates", () => {
		const flag: BooleanFlag = {
			type: "boolean",
			default: false,
			targeting: [
				{
					value: true,
					when: [{ property: "email", op: "ends_with", value: "@acme.com" }],
				},
			],
		};
		expect(
			evaluate(flag, {
				userId: "u_1",
				properties: { email: "ceo@acme.com" },
			}).value,
		).toBe(true);
		expect(
			evaluate(flag, {
				userId: "u_1",
				properties: { email: "ceo@elsewhere.com" },
			}).value,
		).toBe(false);
	});
});

describe("multivariate flag evaluation", () => {
	test("variants distribute by weight", () => {
		const flag: MultivariateFlag = {
			type: "multivariate",
			default: "control",
			variants: [
				{ name: "control", weight: 50 },
				{ name: "treatment", weight: 50 },
			],
		};
		let treatment = 0;
		const N = 10000;
		for (let i = 0; i < N; i++) {
			const r = evaluate(flag, { userId: `u_${i}` });
			if (r.value === "treatment") treatment++;
		}
		const ratio = treatment / N;
		expect(ratio).toBeGreaterThan(0.45);
		expect(ratio).toBeLessThan(0.55);
	});

	test("payload is returned when set", () => {
		const flag: MultivariateFlag = {
			type: "multivariate",
			default: "v1",
			variants: [
				{ name: "v1", weight: 100, payload: { maxTokens: 1000 } },
			],
		};
		const r = evaluate(flag, { userId: "u_1" });
		expect(r.value).toEqual({ maxTokens: 1000 });
	});
});
