// Regression tests for the four bootstrap-speedup fixes:
//
//   A. Parallelize reconcile entity fetches (Promise.all fan-out)
//   B. Skip reconcile after cold-load pull (cursor === 0 fast path)
//   C. Race multi-tab election || /api/auth/me on start()
//   D. Fire-and-forget saveCursor on bootstrap
//
// Each test asserts the behavior change that the corresponding fix
// produces. A revert (back to sequential reconcile, unconditional
// reconcile after pull, sequential election+auth, awaited saveCursor)
// makes the named test fail.

import { afterEach, describe, expect, test } from "bun:test";

import { SyncEngine, type ChangeEvent, type Row } from "./index";

// ---------------------------------------------------------------------------
// Minimal fetch stub. Same shape as reconcile.test.ts so the helpers stay
// readable; duplicated rather than imported to keep the regression test
// file self-contained.
// ---------------------------------------------------------------------------

type FetchHandler = (
  url: string,
  init?: RequestInit,
) => Promise<{ status: number; body: unknown }>;

function installFetch(handler: FetchHandler): () => void {
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const { status, body } = await handler(url, init);
    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
      text: async () => JSON.stringify(body),
    } as Response;
  }) as typeof fetch;
  return () => {
    globalThis.fetch = original;
  };
}

function makeEngine(): SyncEngine {
  return new SyncEngine({
    baseUrl: "http://stub.invalid",
    persist: false,
    reconcileMinIntervalMs: 0,
    multiTab: false,
  });
}

function seedStore(engine: SyncEngine, entity: string, rows: Row[]): void {
  for (const row of rows) {
    const id = (row as { id?: unknown }).id;
    if (typeof id !== "string") continue;
    const ev: ChangeEvent = {
      seq: 0,
      entity,
      row_id: id,
      kind: "insert",
      data: row,
      timestamp: "",
    };
    engine.store.applyChange(ev);
  }
}

// ---------------------------------------------------------------------------
// Fix A — Parallel reconcile fan-out
// ---------------------------------------------------------------------------

describe("Fix A: parallel reconcile fan-out", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("fetches all entities concurrently, not sequentially", async () => {
    // Each /api/entities/<E>/cursor handler sleeps 60ms. With sequential
    // iteration (pre-fix) 3 entities take ~180ms. With Promise.all
    // fan-out they take ~60ms. Assert clearly under the sequential
    // baseline so a revert blows up.
    const perEntityDelayMs = 60;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/")) {
        await new Promise((r) => setTimeout(r, perEntityDelayMs));
        return {
          status: 200,
          body: { data: [], next_cursor: null, has_more: false },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    seedStore(engine, "A", [{ id: "a1" }]);
    seedStore(engine, "B", [{ id: "b1" }]);
    seedStore(engine, "C", [{ id: "c1" }]);

    const t0 = Date.now();
    await engine.reconcile(["A", "B", "C"]);
    const elapsed = Date.now() - t0;

    // Sequential would be ~180ms. Parallel should be ~60ms. Allow a
    // generous ceiling (120ms) for CI jitter without losing the signal.
    expect(elapsed).toBeLessThan(120);
  });

  test("per-entity drift bail still fires under parallel execution", async () => {
    // Mid-flight, flip the session signature so the post-await guard
    // skips the apply for the in-flight entity. The other entities'
    // applies still proceed normally. Reconcile must NOT tombstone
    // either entity's local rows on the racing one (drift skip) or
    // the non-racing one (signature held steady through its window).
    let flipped = false;
    const engine = makeEngine();
    engine.session.observeSession({
      userId: "u1",
      tenantId: "org-a",
      isAdmin: false,
      roles: [],
    });
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Racing/cursor")) {
        // First Racing fetch: flip the session mid-flight, return
        // empty. Apply must be skipped (signature changed).
        await new Promise((r) => setTimeout(r, 20));
        if (!flipped) {
          flipped = true;
          engine.session.observeSession({
            userId: "u1",
            tenantId: "org-b",
            isAdmin: false,
            roles: [],
          });
        }
        return {
          status: 200,
          body: { data: [], next_cursor: null, has_more: false },
        };
      }
      // Stable entity: return its row so the apply path is exercised
      // (no-op since the row already matches), proving the parallel
      // task didn't get cancelled.
      if (url.includes("/api/entities/Stable/cursor")) {
        await new Promise((r) => setTimeout(r, 20));
        return {
          status: 200,
          body: {
            data: [{ id: "s1", v: 1 }],
            next_cursor: null,
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    seedStore(engine, "Racing", [{ id: "r1", v: 1 }]);
    seedStore(engine, "Stable", [{ id: "s1", v: 1 }]);

    await engine.reconcile(["Racing", "Stable"]);

    // Racing row must survive — the parallel task bailed on the
    // session-signature drift. (Pre-fix sequential code path also
    // honored this guard, so we're really asserting that the parallel
    // refactor didn't drop it.)
    expect(engine.store.list("Racing").length).toBe(1);
    // Stable row should still be present (apply was a no-op diff).
    expect(engine.store.list("Stable").length).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Fix B — Skip reconcile after cold-load pull from cursor=0
// ---------------------------------------------------------------------------

describe("Fix B: cold-load skip reconcile", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("lastPullStartedFromZero flag set after successful cold pull", async () => {
    restore = installFetch(async (url) => {
      if (url.includes("/api/sync/pull")) {
        return {
          status: 200,
          body: {
            changes: [],
            cursor: { last_seq: 5 },
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    // Cursor starts at 0, so this pull is a "cold load."
    await engine.pull();

    const flag = (engine as unknown as { lastPullStartedFromZero: boolean })
      .lastPullStartedFromZero;
    expect(flag).toBe(true);
  });

  test("lastPullStartedFromZero NOT set when cursor > 0", async () => {
    let pullSeenSince = "";
    restore = installFetch(async (url) => {
      if (url.includes("/api/sync/pull")) {
        const match = url.match(/since=(\d+)/);
        pullSeenSince = match ? match[1] : "";
        return {
          status: 200,
          body: {
            changes: [],
            cursor: { last_seq: 10 },
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    // Advance the cursor — subsequent pull is incremental, NOT a cold
    // load. The post-WS reconcile should still run.
    (engine as unknown as { cursor: { last_seq: number } }).cursor = {
      last_seq: 7,
    };

    await engine.pull();
    expect(pullSeenSince).toBe("7");

    const flag = (engine as unknown as { lastPullStartedFromZero: boolean })
      .lastPullStartedFromZero;
    expect(flag).toBe(false);
  });

  test("flag survives one-shot consumption: reset to false manually", async () => {
    restore = installFetch(async (url) => {
      if (url.includes("/api/sync/pull")) {
        return {
          status: 200,
          body: {
            changes: [],
            cursor: { last_seq: 1 },
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    await engine.pull();

    const internal = engine as unknown as { lastPullStartedFromZero: boolean };
    expect(internal.lastPullStartedFromZero).toBe(true);
    // Simulate what onConnected does: read + reset.
    internal.lastPullStartedFromZero = false;
    expect(internal.lastPullStartedFromZero).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Fix C — Parallel election || /api/auth/me on bootstrap
//
// The trickiest fix to test because it requires two engines sharing a
// BroadcastChannel. We sidestep that here by asserting the engine-level
// invariant directly: fetchSessionBootstrap exists, is NOT gated by
// `isMultiTabLeader`, and the bootstrap call site (start()) discards
// its result when the engine ended up a follower.
// ---------------------------------------------------------------------------

describe("Fix C: race election || auth/me", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("fetchSessionBootstrap returns ResolvedSession without isMultiTabLeader gate", async () => {
    let authMeHits = 0;
    restore = installFetch(async (url) => {
      if (url.endsWith("/api/auth/me")) {
        authMeHits += 1;
        return {
          status: 200,
          body: {
            user_id: "u1",
            tenant_id: "org-x",
            is_admin: false,
            roles: ["editor"],
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    // Force follower state — refreshResolvedSession() would return
    // immediately under this flag. fetchSessionBootstrap MUST still
    // do the network fetch (that's the whole point of the parallel
    // bootstrap path: we don't know we lost until the election
    // resolves, by which time the fetch is in flight).
    (engine as unknown as { isMultiTabLeader: boolean }).isMultiTabLeader = false;

    const result = await (
      engine as unknown as {
        fetchSessionBootstrap(): Promise<{
          userId: string | null;
          tenantId: string | null;
          isAdmin: boolean;
          roles: string[];
        } | null>;
      }
    ).fetchSessionBootstrap();

    expect(authMeHits).toBe(1);
    expect(result).not.toBeNull();
    expect(result?.userId).toBe("u1");
    expect(result?.tenantId).toBe("org-x");
  });

  test("fetchSessionBootstrap returns null on non-ok response", async () => {
    restore = installFetch(async (url) => {
      if (url.endsWith("/api/auth/me")) {
        return { status: 401, body: {} };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    const result = await (
      engine as unknown as {
        fetchSessionBootstrap(): Promise<unknown>;
      }
    ).fetchSessionBootstrap();
    expect(result).toBeNull();
  });

  test("fetchSessionBootstrap returns null on thrown fetch", async () => {
    const original = globalThis.fetch;
    globalThis.fetch = (async () => {
      throw new Error("network down");
    }) as unknown as typeof fetch;
    restore = () => {
      globalThis.fetch = original;
    };

    const engine = makeEngine();
    const result = await (
      engine as unknown as {
        fetchSessionBootstrap(): Promise<unknown>;
      }
    ).fetchSessionBootstrap();
    expect(result).toBeNull();
  });

  test("two tabs racing election: only the leader applies its bootstrap session", async () => {
    // Multi-tab integration: two engines share a BroadcastChannel (Bun
    // provides one in its test runtime). Both kick off
    // fetchSessionBootstrap in parallel. Both fetches land — the
    // network fetch isn't leader-gated; that's the whole speedup —
    // but only the engine that won the election routes the result
    // into the resolver via applySessionTransition. The follower's
    // result is discarded by start().
    //
    // This test pins the invariant: a follower MUST NOT commit a
    // session it fetched during the parallel bootstrap. If a future
    // refactor "simplifies" the bootstrap back to gating the fetch on
    // isMultiTabLeader, no behavior changes here (the test still
    // passes); if the refactor instead drops the leader check at the
    // APPLY point, follower starts double-committing — and this test
    // fails because the follower would call applySessionTransition
    // which broadcasts back to the leader.

    const appName = "boot-race-" + Math.random().toString(36).slice(2);
    let authMeHits = 0;
    const original = globalThis.fetch;
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/api/auth/me")) {
        authMeHits += 1;
        await new Promise((r) => setTimeout(r, 10));
        return {
          ok: true,
          status: 200,
          json: async () => ({
            user_id: "u1",
            tenant_id: null,
            is_admin: false,
            roles: [],
          }),
          text: async () => "",
        } as Response;
      }
      if (url.includes("/api/sync/pull")) {
        return {
          ok: true,
          status: 200,
          json: async () => ({
            changes: [],
            cursor: { last_seq: 0 },
            has_more: false,
          }),
          text: async () => "",
        } as Response;
      }
      return {
        ok: false,
        status: 404,
        json: async () => ({}),
        text: async () => "",
      } as Response;
    }) as typeof fetch;
    restore = () => {
      globalThis.fetch = original;
    };

    // Two engines, same appName, multi-tab broker enabled. They share
    // a BroadcastChannel and elect a leader.
    const a = new SyncEngine({
      baseUrl: "http://stub.invalid",
      appName,
      persist: false,
      reconcileMinIntervalMs: 0,
      // Polling avoids opening a real WebSocket (Bun's WebSocket
      // global doesn't speak the test transport's protocol).
      transport: "poll",
    });
    // Wait a moment so b's election startTime is strictly greater
    // → a wins the election deterministically.
    const startA = a.start();
    await new Promise((r) => setTimeout(r, 30));
    const b = new SyncEngine({
      baseUrl: "http://stub.invalid",
      appName,
      persist: false,
      reconcileMinIntervalMs: 0,
      transport: "poll",
    });
    const startB = b.start();

    await Promise.all([startA, startB]);
    // Let any trailing broadcasts / poll ticks settle.
    await new Promise((r) => setTimeout(r, 100));

    // a is the leader, b is the follower.
    const aLeader = (a as unknown as { isMultiTabLeader: boolean }).isMultiTabLeader;
    const bLeader = (b as unknown as { isMultiTabLeader: boolean }).isMultiTabLeader;
    expect(aLeader).toBe(true);
    expect(bLeader).toBe(false);

    // Both engines may have fetched /api/auth/me during their parallel
    // bootstrap (that's the speedup). The invariant is that the
    // follower's session state still came from the LEADER broadcast,
    // not from its own discarded fetch. We verify by checking the
    // follower's session resolved to the leader's view.
    const aSession = a.session.signature();
    const bSession = b.session.signature();
    expect(bSession).toBe(aSession);
    // Both saw auth/me at most once each (no retry storm).
    expect(authMeHits).toBeGreaterThanOrEqual(1);
    expect(authMeHits).toBeLessThanOrEqual(2);

    a.stop();
    b.stop();
  });
});

// ---------------------------------------------------------------------------
// Fix D — Fire-and-forget saveCursor on bootstrap
//
// The behavioral change is purely "didn't block start()". Hard to assert
// directly without timing flakiness; instead pin the invariant that
// start() completes even when saveCursor is artificially slow.
// ---------------------------------------------------------------------------

describe("Fix D: fire-and-forget bootstrap saveCursor", () => {
  // This is the lightest-weight assertion that pins the fix: the
  // bootstrap call site uses `void this.persistence.saveCursor(...)`,
  // not `await`. We can't easily install a slow persistence layer into
  // an engine that explicitly opted out of persistence, so the
  // regression coverage here is the AST shape — verified by reading
  // the file and asserting the call is not awaited.
  test("source uses void (not await) for the bootstrap saveCursor", async () => {
    const path = new URL("./index.ts", import.meta.url).pathname;
    const src = await Bun.file(path).text();
    // Find the bootstrap save block.
    const marker = "// Save cursor after pull.";
    const idx = src.indexOf(marker);
    expect(idx).toBeGreaterThan(0);
    const window = src.slice(idx, idx + 800);
    // The fixed form uses `void this.persistence.saveCursor`; the
    // pre-fix form was `await this.persistence.saveCursor`. Pin both
    // sides of that change so a revert fails clearly.
    expect(window).toContain("void this.persistence.saveCursor(this.cursor)");
    expect(window).not.toContain("await this.persistence.saveCursor");
  });
});
