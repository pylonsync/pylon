// Regression coverage for the useRoom refcounted room registry.
//
// The chat app mounts useRoom from 3+ components per channel (presence
// count, presence avatars, composer). Without deduping, that produced N
// joins + N heartbeats + N leaves per channel view. These tests pin the
// contract that the module-scoped registry collapses identical
// (baseUrl, roomId, userId, token) tuples into a single network
// footprint regardless of how many React subscribers there are.
//
// We drive the registry directly via `__roomRegistryInternals` instead
// of mounting React because the react package doesn't ship a renderer
// dep. The registry IS the contract under test — the hook is a thin
// subscriber on top of it.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { __roomRegistryInternals } from "./useRoom";

// --- fetch stub --------------------------------------------------------

interface RecordedRequest {
  url: string;
  method: string;
  body: any;
}

let recorded: RecordedRequest[];
let originalFetch: typeof globalThis.fetch;

function installFetchStub(): void {
  recorded = [];
  originalFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    let body: any = null;
    if (typeof init?.body === "string") {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    }
    recorded.push({ url, method, body });
    // All room endpoints return JSON; the snapshot/members shape is what
    // useRoom unpacks. An empty object is enough to exercise the
    // "joined" success path without inventing peers.
    const responseBody = JSON.stringify({ snapshot: { peers: [] }, members: [] });
    return new Response(responseBody, {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

function restoreFetch(): void {
  globalThis.fetch = originalFetch;
}

function count(predicate: (r: RecordedRequest) => boolean): number {
  return recorded.filter(predicate).length;
}

function isJoin(r: RecordedRequest): boolean {
  return r.method === "POST" && r.url.endsWith("/api/rooms/join");
}
function isLeave(r: RecordedRequest): boolean {
  return r.method === "POST" && r.url.endsWith("/api/rooms/leave");
}
function isHeartbeat(r: RecordedRequest): boolean {
  return r.method === "GET" && /\/api\/rooms\/[^/]+$/.test(r.url);
}

// Wait until `pred()` returns true, polling on a microtask cadence. The
// join + leave paths are promise-chained off pylonFetch, so a couple of
// microtask flushes is all we need.
async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
}

// --- harness -----------------------------------------------------------

const BASE = "http://stub.invalid";
const ROOM = "room-1";
const USER = "user-1";
const TOKEN = "tok-1";

beforeEach(() => {
  installFetchStub();
  __roomRegistryInternals.reset();
});

afterEach(() => {
  __roomRegistryInternals.reset();
  restoreFetch();
});

// --- tests -------------------------------------------------------------

describe("useRoom shared registry", () => {
  test("three hooks for the same room → 1 join, 1 leave, N (not 3N) heartbeats", async () => {
    // Use a short interval so the test runs fast — semantics are the same.
    const interval = 25;

    const r1 = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);
    const r2 = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);
    const r3 = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);

    // All three resolved to the SAME shared room instance.
    expect(r1).toBe(r2);
    expect(r2).toBe(r3);
    expect(__roomRegistryInternals.size()).toBe(1);

    await flush();
    // Exactly one join across 3 mounts.
    expect(count(isJoin)).toBe(1);

    // Let the heartbeat fire a few times.
    await new Promise((res) => setTimeout(res, interval * 3 + 10));
    const heartbeatsAfter3Ticks = count(isHeartbeat);
    // 3 hooks × 3 ticks would be 9. Shared registry should land at ~3
    // (one per tick). Allow off-by-one for scheduling jitter.
    expect(heartbeatsAfter3Ticks).toBeGreaterThanOrEqual(2);
    expect(heartbeatsAfter3Ticks).toBeLessThanOrEqual(4);

    // Release all three. The first two are no-ops (refcount → 2, → 1);
    // the third schedules teardown.
    __roomRegistryInternals.release(r1);
    __roomRegistryInternals.release(r2);
    expect(count(isLeave)).toBe(0); // no leave while refs still > 0
    __roomRegistryInternals.release(r3);

    // Teardown is deferred via setTimeout(0); wait for it.
    await new Promise((res) => setTimeout(res, 10));
    await flush();

    expect(count(isLeave)).toBe(1);
    expect(__roomRegistryInternals.size()).toBe(0);

    // Heartbeats stop after teardown — capture, wait, verify no growth.
    const heartbeatsAtTeardown = count(isHeartbeat);
    await new Promise((res) => setTimeout(res, interval * 2 + 10));
    expect(count(isHeartbeat)).toBe(heartbeatsAtTeardown);
  });

  test("StrictMode double-mount race: release+acquire in same tick keeps room alive", async () => {
    const interval = 1_000; // long — we don't care about heartbeats here.
    const a = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);
    await flush();
    expect(count(isJoin)).toBe(1);

    // StrictMode cleanup: refcount → 0, teardown SCHEDULED but not fired.
    __roomRegistryInternals.release(a);
    // Synchronous remount lands before the setTimeout(0) callback runs.
    const b = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);
    expect(b).toBe(a); // cancelled teardown — same shared room

    // Wait long enough for any pending teardown to have fired.
    await new Promise((res) => setTimeout(res, 20));
    await flush();

    // No leave ever shipped — the room stayed alive across the cycle.
    expect(count(isLeave)).toBe(0);
    expect(count(isJoin)).toBe(1); // and no extra join either

    __roomRegistryInternals.release(b);
    await new Promise((res) => setTimeout(res, 20));
    await flush();
    expect(count(isLeave)).toBe(1);
  });

  test("different room ids do NOT dedupe (separate joins, separate leaves)", async () => {
    const a = __roomRegistryInternals.acquire(BASE, "room-A", USER, TOKEN, {}, 1_000);
    const b = __roomRegistryInternals.acquire(BASE, "room-B", USER, TOKEN, {}, 1_000);
    expect(a).not.toBe(b);
    expect(__roomRegistryInternals.size()).toBe(2);

    await flush();
    expect(count(isJoin)).toBe(2);

    __roomRegistryInternals.release(a);
    __roomRegistryInternals.release(b);
    await new Promise((res) => setTimeout(res, 20));
    await flush();
    expect(count(isLeave)).toBe(2);
  });

  test("different tokens do NOT dedupe (auth context differs)", async () => {
    const a = __roomRegistryInternals.acquire(BASE, ROOM, USER, "tok-A", {}, 1_000);
    const b = __roomRegistryInternals.acquire(BASE, ROOM, USER, "tok-B", {}, 1_000);
    expect(a).not.toBe(b);
    expect(__roomRegistryInternals.size()).toBe(2);

    await flush();
    expect(count(isJoin)).toBe(2);
  });

  test("acquire+release before join resolves still pairs join with leave once join lands", async () => {
    // Microtask ordering: the join fetch resolves before the setTimeout(0)
    // teardown fires, so even an immediate release waits for the join
    // landing before shipping leave. That's the contract — every join
    // we ship gets paired with a leave (server is idempotent regardless).
    const interval = 1_000;
    const a = __roomRegistryInternals.acquire(BASE, ROOM, USER, TOKEN, {}, interval);
    __roomRegistryInternals.release(a);

    await new Promise((res) => setTimeout(res, 20));
    await flush();

    expect(__roomRegistryInternals.size()).toBe(0);
    expect(count(isJoin)).toBe(1);
    expect(count(isLeave)).toBe(1);
  });
});
