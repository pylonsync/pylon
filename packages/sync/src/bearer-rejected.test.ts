// A stored token the server rejects is cleared. The server answers a request
// whose bearer token resolved to no one (expired, revoked, or not a session
// of this app) with `X-Pylon-Bearer-Rejected: 1`, and falls back to the
// session cookie when it can. Without the client clearing the token, every
// later request keeps sending it.

import { afterEach, describe, expect, test } from "bun:test";

import {
  BEARER_REJECTED_HEADER,
  clearRejectedToken,
  pylonFetch,
  pylonFetchRaw,
  SyncEngine,
} from "./index";
import type { Storage } from "./storage";

function memoryStorage(initial: Record<string, string> = {}): Storage & {
  data: Map<string, string>;
} {
  const data = new Map(Object.entries(initial));
  return {
    data,
    get: (k) => data.get(k) ?? null,
    set: (k, v) => void data.set(k, v),
    remove: (k) => void data.delete(k),
  };
}

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

/** Stub fetch: records each request's Authorization header and answers
 *  with `X-Pylon-Bearer-Rejected: 1` while `rejected(token)` is true. */
function stubFetch(rejected: (token: string | undefined) => boolean, body: unknown = {}) {
  const auth: Array<string | undefined> = [];
  globalThis.fetch = (async (_input: RequestInfo | URL, init: RequestInit = {}) => {
    const h = (init.headers ?? {}) as Record<string, string>;
    const header = h.Authorization;
    auth.push(header);
    const token = header?.replace(/^Bearer /, "");
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (rejected(token)) headers[BEARER_REJECTED_HEADER] = "1";
    return new Response(JSON.stringify(body), { status: 200, headers });
  }) as typeof fetch;
  return auth;
}

describe("transport: X-Pylon-Bearer-Rejected", () => {
  test("pylonFetch reports the token it sent when the server rejects it", async () => {
    stubFetch(() => true);
    const seen: string[] = [];
    await pylonFetch(
      { baseUrl: "http://t.invalid", token: "stale", onBearerRejected: (t) => seen.push(t) },
      "/api/fn/x",
      { method: "POST", json: {} },
    );
    expect(seen).toEqual(["stale"]);
  });

  test("pylonFetchRaw reports it too", async () => {
    stubFetch(() => true);
    const seen: string[] = [];
    await pylonFetchRaw(
      { baseUrl: "http://t.invalid", token: "stale", onBearerRejected: (t) => seen.push(t) },
      "/api/fn/x",
      { method: "POST", json: {} },
    );
    expect(seen).toEqual(["stale"]);
  });

  test("nothing is reported without the header, or when no token was sent", async () => {
    const seen: string[] = [];
    stubFetch(() => false);
    await pylonFetch(
      { baseUrl: "http://t.invalid", token: "good", onBearerRejected: (t) => seen.push(t) },
      "/api/auth/me",
    );
    stubFetch(() => true);
    await pylonFetch(
      { baseUrl: "http://t.invalid", onBearerRejected: (t) => seen.push(t) },
      "/api/auth/me",
    );
    expect(seen).toEqual([]);
  });
});

describe("clearRejectedToken", () => {
  test("removes the app's token and user id when the key still holds that token", () => {
    const s = memoryStorage({ "pylon:miles:token": "stale", "pylon:miles:userId": "u1" });
    expect(clearRejectedToken(s, "pylon:miles:token", "stale")).toBe(true);
    expect(s.data.size).toBe(0);
  });

  test("keeps a token that a sign-in wrote while the request was in flight", () => {
    const s = memoryStorage({ "pylon:miles:token": "fresh" });
    expect(clearRejectedToken(s, "pylon:miles:token", "stale")).toBe(false);
    expect(s.get("pylon:miles:token")).toBe("fresh");
  });

  test("leaves the shared default key alone", () => {
    const s = memoryStorage({ pylon_token: "stale" });
    expect(clearRejectedToken(s, "pylon_token", "stale")).toBe(false);
    expect(s.get("pylon_token")).toBe("stale");
  });
});

describe("SyncEngine clears a stored token the server rejects", () => {
  function engine(storage: Storage, extra: Record<string, unknown> = {}) {
    return new SyncEngine({
      baseUrl: "http://t.invalid",
      appName: "miles",
      storage,
      persist: false,
      multiTab: false,
      transport: "poll",
      ...extra,
    });
  }

  test("fn(): the next call sends no Authorization header", async () => {
    const storage = memoryStorage({ "pylon:miles:token": "stale" });
    const auth = stubFetch((t) => t === "stale", { ok: true });
    const e = engine(storage);
    await e.fn("resetDemoDealership", {});
    expect(storage.get("pylon:miles:token")).toBeNull();
    await e.fn("resetDemoDealership", {});
    expect(auth).toEqual(["Bearer stale", undefined]);
  });

  test("a token the server accepts stays", async () => {
    const storage = memoryStorage({ "pylon:miles:token": "good" });
    stubFetch(() => false, { ok: true });
    await engine(storage).fn("resetDemoDealership", {});
    expect(storage.get("pylon:miles:token")).toBe("good");
  });

  test("an explicitly configured token is the caller's and stays", async () => {
    const storage = memoryStorage({ "pylon:miles:token": "stale" });
    stubFetch(() => true, { ok: true });
    await engine(storage, { token: "stale" }).fn("resetDemoDealership", {});
    expect(storage.get("pylon:miles:token")).toBe("stale");
  });
});
