import { describe, expect, test } from "bun:test";

import { entitlementsFromSubscriber } from "./handlers/sync";
import { entitlementIdsOf, revenuecatWebhookHandler, statusForEvent, webhookAuthorized } from "./handlers/webhook";
import { hasEntitlement } from "./index";
import type { HandlerCtx } from "./types";

describe("event mapping", () => {
	test("purchase-like events are active, expiration and refund end access, cancellation keeps it", () => {
		for (const t of ["INITIAL_PURCHASE", "RENEWAL", "NON_RENEWING_PURCHASE", "UNCANCELLATION", "PRODUCT_CHANGE", "BILLING_ISSUE", "CANCELLATION"]) {
			expect(statusForEvent(t)).toBe("active");
		}
		expect(statusForEvent("EXPIRATION")).toBe("expired");
		expect(statusForEvent("REFUND")).toBe("expired");
		expect(statusForEvent("TEST")).toBeNull();
		expect(statusForEvent("TRANSFER")).toBeNull();
	});

	test("entitlement ids come from the list, then the single field", () => {
		expect(entitlementIdsOf({ type: "RENEWAL", entitlement_ids: ["pro", "coach"] })).toEqual(["pro", "coach"]);
		expect(entitlementIdsOf({ type: "RENEWAL", entitlement_id: "pro" })).toEqual(["pro"]);
		expect(entitlementIdsOf({ type: "RENEWAL" })).toEqual([]);
	});

	test("the shared secret is required and accepted bare or as a bearer", () => {
		expect(webhookAuthorized("s3cret", "s3cret")).toBe(true);
		expect(webhookAuthorized("Bearer s3cret", "s3cret")).toBe(true);
		expect(webhookAuthorized("wrong", "s3cret")).toBe(false);
		expect(webhookAuthorized("s3cret", undefined)).toBe(false);
		expect(webhookAuthorized(undefined, "s3cret")).toBe(false);
	});

	test("subscriber payloads map to active or expired by expires_date", () => {
		const now = Date.parse("2026-09-05T00:00:00Z");
		const rows = entitlementsFromSubscriber(
			{
				subscriber: {
					entitlements: {
						pro: { expires_date: "2026-10-01T00:00:00Z", product_identifier: "pro_monthly" },
						coach: { expires_date: "2026-01-01T00:00:00Z", product_identifier: "coach_10" },
						lifetime: { expires_date: null, product_identifier: "lifetime" },
					},
				},
			},
			now,
		);
		expect(rows.map((r) => [r.entitlement, r.status])).toEqual([
			["pro", "active"],
			["coach", "expired"],
			["lifetime", "active"],
		]);
	});

	test("hasEntitlement ignores rows whose expiry has passed", () => {
		const rows = [
			{ entitlement: "pro", status: "active", expiresAt: "2026-01-01T00:00:00Z" },
			{ entitlement: "coach", status: "active", expiresAt: null },
		];
		const now = Date.parse("2026-09-05T00:00:00Z");
		expect(hasEntitlement(rows, "pro", now)).toBe(false);
		expect(hasEntitlement(rows, "coach", now)).toBe(true);
		expect(hasEntitlement(rows, "missing", now)).toBe(false);
	});
});

function ctxWith(headers: Record<string, string>, body: unknown, env: Record<string, string>) {
	const mutations: Array<Record<string, unknown>> = [];
	const ctx = {
		env,
		auth: { userId: null },
		request: { headers, rawBody: JSON.stringify(body) },
		error: (code: string, message: string) => Object.assign(new Error(message), { code }),
		runQuery: async () => [] as never,
		runMutation: async (_name: string, args: Record<string, unknown>) => {
			mutations.push(args);
			return { changed: true } as never;
		},
	} as unknown as HandlerCtx;
	return { ctx, mutations };
}

describe("webhook handler", () => {
	const handler = revenuecatWebhookHandler({ entitlements: ["pro"] }) as unknown as {
		handler: (ctx: HandlerCtx, args: Record<string, never>) => Promise<{ applied: number; skipped?: string }>;
	};

	test("refuses when the secret is unset or wrong", async () => {
		const unset = ctxWith({ authorization: "x" }, { event: { type: "RENEWAL" } }, {});
		await expect(handler.handler(unset.ctx, {})).rejects.toThrow("REVENUECAT_WEBHOOK_AUTH");
		const wrong = ctxWith({ authorization: "nope" }, { event: { type: "RENEWAL" } }, { REVENUECAT_WEBHOOK_AUTH: "s3cret" });
		await expect(handler.handler(wrong.ctx, {})).rejects.toThrow("bad webhook authorization");
		expect(wrong.mutations).toHaveLength(0);
	});

	test("writes one active row per entitlement on a purchase", async () => {
		const { ctx, mutations } = ctxWith(
			{ authorization: "s3cret" },
			{
				event: {
					type: "INITIAL_PURCHASE",
					app_user_id: "user_1",
					product_id: "pro_monthly",
					entitlement_ids: ["pro"],
					store: "APP_STORE",
					environment: "SANDBOX",
					expiration_at_ms: Date.parse("2026-10-05T00:00:00Z"),
				},
			},
			{ REVENUECAT_WEBHOOK_AUTH: "s3cret" },
		);
		const out = await handler.handler(ctx, {});
		expect(out.applied).toBe(1);
		expect(mutations[0]).toMatchObject({
			userId: "user_1",
			entitlement: "pro",
			productId: "pro_monthly",
			status: "active",
			store: "app_store",
			environment: "SANDBOX",
			expiresAt: "2026-10-05T00:00:00.000Z",
		});
	});

	test("anonymous subscribers are acknowledged and ignored", async () => {
		const { ctx, mutations } = ctxWith(
			{ authorization: "s3cret" },
			{ event: { type: "INITIAL_PURCHASE", app_user_id: "$RCAnonymousID:abc", entitlement_ids: ["pro"] } },
			{ REVENUECAT_WEBHOOK_AUTH: "s3cret" },
		);
		const out = await handler.handler(ctx, {});
		expect(out.skipped).toBe("no app user id");
		expect(mutations).toHaveLength(0);
	});
});
