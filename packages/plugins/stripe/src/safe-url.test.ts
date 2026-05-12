import { expect, test } from "bun:test";

import { assertSafeRedirectUrl } from "./safe-url";

test("accepts URL on the PYLON_PUBLIC_URL host", () => {
	expect(() =>
		assertSafeRedirectUrl(
			"https://api.getyapless.com/dashboard?welcome=1",
			"https://api.getyapless.com",
		),
	).not.toThrow();
});

test("accepts subdomain of the PYLON_PUBLIC_URL host", () => {
	// PYLON_PUBLIC_URL=https://api.acme.com → www.acme.com allowed.
	// (The check is suffix-based on the trailing dot, so api.acme.com
	// allows *.acme.com? — we actually allow only hosts ending in
	// `.<host>`, so we accept "ext.api.acme.com" but NOT "acme.com"
	// or "www.acme.com". This is intentional — the SyncProjectCors
	// helper in pylon-cloud emits the apex/www variants explicitly.)
	expect(() =>
		assertSafeRedirectUrl(
			"https://ext.api.getyapless.com/cb",
			"https://api.getyapless.com",
		),
	).not.toThrow();
});

test("rejects unrelated host", () => {
	expect(() =>
		assertSafeRedirectUrl(
			"https://attacker.example.com/steal",
			"https://api.getyapless.com",
		),
	).toThrow(/not on the allowed host/);
});

test("accepts localhost in dev", () => {
	expect(() =>
		assertSafeRedirectUrl("http://localhost:3000/cb", "https://api.x.com"),
	).not.toThrow();
});

test("explicit extraHost widens the allowlist", () => {
	expect(() =>
		assertSafeRedirectUrl(
			"https://www.getyapless.com/dashboard",
			"https://api.getyapless.com",
			{ extraHost: "www.getyapless.com" },
		),
	).not.toThrow();
});

test("extraOrigins (PYLON_CORS_ORIGIN-style) parses comma list", () => {
	expect(() =>
		assertSafeRedirectUrl(
			"https://www.getyapless.com/dashboard",
			"https://api.getyapless.com",
			{
				extraOrigins:
					"https://getyapless.com,https://www.getyapless.com,https://pylon-yapless.fly.dev",
			},
		),
	).not.toThrow();
});

test("malformed URLs throw a clear error", () => {
	expect(() => assertSafeRedirectUrl("not-a-url", "https://x.com")).toThrow(
		/invalid URL/,
	);
});
