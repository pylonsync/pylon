import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { SyncEngine, type ChangeEvent, type Row } from "./index";

// ---------------------------------------------------------------------------
// Minimal fetch stub + in-memory persistence shim so the tests exercise
// the reconcile code path without a real browser environment.
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
  // `persist: false` short-circuits the IndexedDB import — these tests
  // run on the Bun runtime which has no `indexedDB` global. Reconcile
  // itself only touches `this.persistence` defensively, so disabling
  // the layer is harmless here.
  return new SyncEngine({
    baseUrl: "http://stub.invalid",
    persist: false,
    reconcileMinIntervalMs: 0,
  });
}

// Seed rows directly into the in-memory store (bypassing the WS / pull
// path so the test isolates `reconcile`'s diff semantics).
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

describe("SyncEngine.reconcile", () => {
  let restore: (() => void) | null = null;

  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("removes local rows the server no longer returns (Repro A)", async () => {
    // Server truth: only row "r1" survives.
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Recording/cursor")) {
        return {
          status: 200,
          body: {
            data: [{ id: "r1", title: "alive" }],
            next_cursor: null,
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    seedStore(engine, "Recording", [
      { id: "r1", title: "alive" },
      { id: "r2", title: "phantom" },
      { id: "r3", title: "phantom" },
    ]);

    expect(engine.store.list("Recording").length).toBe(3);

    await engine.reconcile(["Recording"]);

    const remaining = engine.store.list("Recording");
    expect(remaining.length).toBe(1);
    expect((remaining[0] as { id: string }).id).toBe("r1");
  });

  test("updates local rows whose content drifted from server (Repro B)", async () => {
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Org/cursor")) {
        return {
          status: 200,
          body: {
            data: [
              {
                id: "org_1",
                plan: "lifetime",
                subscriptionStatus: "active",
              },
            ],
            next_cursor: null,
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    seedStore(engine, "Org", [
      { id: "org_1", plan: "pending", subscriptionStatus: "incomplete" },
    ]);

    await engine.reconcile(["Org"]);

    const row = engine.store.get("Org", "org_1") as {
      plan: string;
      subscriptionStatus: string;
    } | null;
    expect(row).not.toBeNull();
    expect(row!.plan).toBe("lifetime");
    expect(row!.subscriptionStatus).toBe("active");
  });

  test("inserts server rows the local replica is missing", async () => {
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Recording/cursor")) {
        return {
          status: 200,
          body: {
            data: [
              { id: "r1", title: "one" },
              { id: "r2", title: "two" },
            ],
            next_cursor: null,
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    seedStore(engine, "Recording", [{ id: "r1", title: "one" }]);

    await engine.reconcile(["Recording"]);

    expect(engine.store.list("Recording").length).toBe(2);
    expect(engine.store.get("Recording", "r2")).not.toBeNull();
  });

  test("removed rows record a tombstone bound to the cursor, not MAX_SAFE_INTEGER", async () => {
    // Server has nothing — every local row should be dropped.
    restore = installFetch(async () => ({
      status: 200,
      body: { data: [], next_cursor: null, has_more: false },
    }));

    const engine = makeEngine();
    seedStore(engine, "Recording", [{ id: "r_ghost", title: "phantom" }]);
    await engine.reconcile(["Recording"]);
    expect(engine.store.list("Recording").length).toBe(0);

    // A server-broadcast insert that arrives AFTER reconcile must be
    // accepted — the tombstone is bound to the current cursor (0 here),
    // and the new event has seq > 0 so it bypasses the tombstone check.
    // Without the cursor-bound tombstone (e.g. MAX_SAFE_INTEGER), the
    // row could never come back even after legitimate server-side
    // re-creation.
    engine.store.applyChange({
      seq: 1,
      entity: "Recording",
      row_id: "r_ghost",
      kind: "insert",
      data: { id: "r_ghost", title: "recreated" },
      timestamp: "",
    });
    expect(engine.store.get("Recording", "r_ghost")).not.toBeNull();
  });

  test("no-op when no entities have local rows", async () => {
    let calls = 0;
    restore = installFetch(async () => {
      calls += 1;
      return { status: 200, body: { data: [], next_cursor: null, has_more: false } };
    });

    const engine = makeEngine();
    await engine.reconcile(); // unscoped — should find no entities to check
    expect(calls).toBe(0);
  });

  test("debounces back-to-back unscoped calls", async () => {
    let calls = 0;
    restore = installFetch(async () => {
      calls += 1;
      return {
        status: 200,
        body: {
          data: [{ id: "r1", title: "alive" }],
          next_cursor: null,
          has_more: false,
        },
      };
    });

    // Engine with a 5s debounce — reconcile twice in a row should only
    // hit the network once.
    const engine = new SyncEngine({
      baseUrl: "http://stub.invalid",
      persist: false,
      reconcileMinIntervalMs: 5_000,
    });
    seedStore(engine, "Recording", [{ id: "r1", title: "alive" }]);

    await engine.reconcile();
    await engine.reconcile();
    expect(calls).toBe(1);
  });

  test("404 on entity → drops every local row for it", async () => {
    restore = installFetch(async () => ({ status: 404, body: {} }));

    const engine = makeEngine();
    seedStore(engine, "DeletedEntity", [{ id: "x", value: 1 }]);
    await engine.reconcile(["DeletedEntity"]);
    expect(engine.store.list("DeletedEntity").length).toBe(0);
  });
});

describe("LocalStore.reconcileRemove", () => {
  test("returns true when the row existed", () => {
    const engine = makeEngine();
    seedStore(engine, "Doc", [{ id: "d1" }]);
    expect(engine.store.reconcileRemove("Doc", "d1", 10)).toBe(true);
    expect(engine.store.get("Doc", "d1")).toBeNull();
  });

  test("returns false when the row didn't exist (no-op)", () => {
    const engine = makeEngine();
    expect(engine.store.reconcileRemove("Doc", "missing", 10)).toBe(false);
  });

  test("tombstone seq bounded by argument, not MAX_SAFE_INTEGER", () => {
    const engine = makeEngine();
    seedStore(engine, "Doc", [{ id: "d1" }]);
    engine.store.reconcileRemove("Doc", "d1", 50);

    // Insert with seq < tombstone → dropped.
    engine.store.applyChange({
      seq: 30,
      entity: "Doc",
      row_id: "d1",
      kind: "insert",
      data: { id: "d1" },
      timestamp: "",
    });
    expect(engine.store.get("Doc", "d1")).toBeNull();

    // Insert with seq > tombstone → accepted.
    engine.store.applyChange({
      seq: 100,
      entity: "Doc",
      row_id: "d1",
      kind: "insert",
      data: { id: "d1", value: "recreated" },
      timestamp: "",
    });
    expect(engine.store.get("Doc", "d1")).not.toBeNull();
  });
});

describe("LocalStore.entityNames", () => {
  test("returns only entities with at least one row", () => {
    const engine = makeEngine();
    seedStore(engine, "A", [{ id: "a1" }]);
    seedStore(engine, "B", [{ id: "b1" }]);
    seedStore(engine, "C", []);
    const names = engine.store.entityNames().sort();
    expect(names).toEqual(["A", "B"]);
  });
});
