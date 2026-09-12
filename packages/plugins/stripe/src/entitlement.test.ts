import { describe, expect, test } from "bun:test";
import { hasStripeSubscription } from "./entitlement";

const NOW = Date.parse("2026-09-12T00:00:00Z");
const future = "2026-10-01T00:00:00Z";
const past = "2026-09-01T00:00:00Z";

describe("stripe entitlement, read from synced rows", () => {
	test("an active subscription inside its period carries access", () => {
		expect(hasStripeSubscription([{ status: "active", currentPeriodEnd: future }], NOW)).toBe(true);
	});

	test("a trial is access", () => {
		expect(hasStripeSubscription([{ status: "trialing", currentPeriodEnd: future }], NOW)).toBe(
			true,
		);
	});

	test("past_due keeps access until the paid period actually ends", () => {
		// Every card retry takes days. Cutting a paying customer off over a
		// payment that will clear tomorrow costs more than the days it saves.
		expect(hasStripeSubscription([{ status: "past_due", currentPeriodEnd: future }], NOW)).toBe(
			true,
		);
		expect(hasStripeSubscription([{ status: "past_due", currentPeriodEnd: past }], NOW)).toBe(false);
	});

	test("an unpaid first invoice is not access", () => {
		// `incomplete` is how someone would otherwise subscribe by abandoning
		// a card form.
		expect(hasStripeSubscription([{ status: "incomplete", currentPeriodEnd: future }], NOW)).toBe(
			false,
		);
	});

	test("an elapsed period is inactive even before the webhook says so", () => {
		expect(hasStripeSubscription([{ status: "active", currentPeriodEnd: past }], NOW)).toBe(false);
	});

	test("a row with no period end grants nothing", () => {
		// An undated subscription is a bug upstream; granting on it forever is
		// the wrong way to find out.
		expect(hasStripeSubscription([{ status: "active" }], NOW)).toBe(false);
		expect(hasStripeSubscription([{ status: "active", currentPeriodEnd: null }], NOW)).toBe(false);
		expect(hasStripeSubscription([{ status: "active", currentPeriodEnd: "soon" }], NOW)).toBe(false);
	});

	test("a lapsed row does not hide a live one", () => {
		expect(
			hasStripeSubscription(
				[
					{ status: "canceled", currentPeriodEnd: past },
					{ status: "active", currentPeriodEnd: future },
				],
				NOW,
			),
		).toBe(true);
	});

	test("no rows is no access", () => {
		expect(hasStripeSubscription([], NOW)).toBe(false);
	});
});
