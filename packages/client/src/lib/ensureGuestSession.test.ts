// Two <EnsureGuest> mounts (or StrictMode's double effect) called
// ensureGuestSession() at the same time; both saw no token and each minted
// its own guest, so the browser ended up with two identities and the second
// overwrote the first's token.

import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

const stored = new Map<string, string>();
let sessionRefreshes = 0;

mock.module("@pylonsync/react", () => ({
	db: {
		sync: {
			notifySessionChanged: async () => {
				sessionRefreshes += 1;
			},
		},
	},
	getBaseUrl: () => "http://test.invalid",
	storageKey: (k: string) => `pylon:test:${k}`,
}));

const { ensureGuestSession } = await import("./api");

const realFetch = globalThis.fetch;
const realWindow = (globalThis as { window?: unknown }).window;
let guestRequests = 0;

beforeEach(() => {
	stored.clear();
	sessionRefreshes = 0;
	guestRequests = 0;
	(globalThis as { window?: unknown }).window = {
		localStorage: {
			getItem: (k: string) => stored.get(k) ?? null,
			setItem: (k: string, v: string) => void stored.set(k, v),
			removeItem: (k: string) => void stored.delete(k),
		},
	};
	globalThis.fetch = (async (url: string) => {
		if (String(url).endsWith("/api/auth/guest")) {
			guestRequests += 1;
			const n = guestRequests;
			await new Promise((r) => setTimeout(r, 5));
			return new Response(JSON.stringify({ token: `tok-${n}`, user_id: `guest_${n}` }), {
				status: 200,
				headers: { "content-type": "application/json" },
			});
		}
		return new Response("{}", { status: 404 });
	}) as typeof fetch;
});

afterEach(() => {
	globalThis.fetch = realFetch;
	(globalThis as { window?: unknown }).window = realWindow;
});

describe("ensureGuestSession", () => {
	test("concurrent calls mint one guest session", async () => {
		const [a, b, c] = await Promise.all([
			ensureGuestSession(),
			ensureGuestSession(),
			ensureGuestSession(),
		]);
		expect(guestRequests).toBe(1);
		expect(a?.user_id).toBe("guest_1");
		expect(b?.user_id).toBe("guest_1");
		expect(c?.user_id).toBe("guest_1");
		expect(stored.get("pylon:test:token")).toBe("tok-1");
		expect(sessionRefreshes).toBe(1);
	});

	test("a call after the session exists does not mint again", async () => {
		await ensureGuestSession();
		expect(await ensureGuestSession()).toBeNull();
		expect(guestRequests).toBe(1);
	});

	test("a failed mint does not block a later retry", async () => {
		globalThis.fetch = (async () => {
			guestRequests += 1;
			return new Response("{}", { status: 503 });
		}) as unknown as typeof fetch;
		expect(await ensureGuestSession()).toBeNull();
		expect(await ensureGuestSession()).toBeNull();
		expect(guestRequests).toBe(2);
	});
});
