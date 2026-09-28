// Concurrent guestSession() calls each minted their own guest, so a device
// could end up with two identities and the later token overwrote the first.

import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

const stored = new Map<string, string>();

mock.module("@pylonsync/react", () => ({
  db: { sync: { notifySessionChanged: async () => {} } },
  getBaseUrl: () => "http://test.invalid",
  getReactStorage: () => ({
    get: (k: string) => stored.get(k) ?? null,
    set: (k: string, v: string) => void stored.set(k, v),
    remove: (k: string) => void stored.delete(k),
  }),
  storageKey: (k: string) => `pylon:test:${k}`,
}));

const { guestSession } = await import("./auth");

const realFetch = globalThis.fetch;
let guestRequests = 0;

beforeEach(() => {
  stored.clear();
  guestRequests = 0;
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
});

describe("guestSession", () => {
  test("concurrent calls share one request", async () => {
    const [a, b] = await Promise.all([guestSession(), guestSession()]);
    expect(guestRequests).toBe(1);
    expect(a.token).toBe("tok-1");
    expect(b.token).toBe("tok-1");
    expect(stored.get("pylon:test:token")).toBe("tok-1");
  });

  test("a later call after the first settles makes a new request", async () => {
    await guestSession();
    await guestSession();
    expect(guestRequests).toBe(2);
  });
});
