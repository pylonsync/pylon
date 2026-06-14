// Regression: `getBaseUrl()` is the origin every @pylonsync/client auth helper
// (createOrg, passwordRegister, createInvite, …) and the room/shard hooks fetch
// against. It used to return a static `http://localhost:4321` until an explicit
// `configureClient({ baseUrl })` — so a unified same-origin SSR app (which never
// calls configureClient) fired all auth/org calls at the dev port: broken on any
// non-4321 dev port AND in production. The fix defaults to the page origin in a
// browser. These pin that contract.

import { afterEach, describe, expect, test } from "bun:test";

import { configureClient, getBaseUrl } from "./index";

const realWindow = (globalThis as { window?: unknown }).window;
afterEach(() => {
  (globalThis as { window?: unknown }).window = realWindow;
});

describe("getBaseUrl() — same-origin default for unified SSR apps", () => {
  // Run order matters: these three execute before any configureClient, so the
  // first two observe the un-configured resolution path.
  test("SSR/node (no window) keeps the localhost dev default", () => {
    (globalThis as { window?: unknown }).window = undefined;
    expect(getBaseUrl()).toBe("http://localhost:4321");
  });

  test("browser, unconfigured → the page origin, not localhost:4321", () => {
    (globalThis as { window?: unknown }).window = {
      location: { origin: "https://app.example.com" },
    };
    expect(getBaseUrl()).toBe("https://app.example.com");
  });

  test("an explicit configureClient({ baseUrl }) wins over the origin default", () => {
    (globalThis as { window?: unknown }).window = {
      location: { origin: "https://app.example.com" },
    };
    configureClient({ baseUrl: "https://api.example.com" });
    expect(getBaseUrl()).toBe("https://api.example.com");
    // …and still wins when server-rendered (no window) — a separate-origin API
    // deploy must not silently flip to the page origin.
    (globalThis as { window?: unknown }).window = undefined;
    expect(getBaseUrl()).toBe("https://api.example.com");
  });
});
