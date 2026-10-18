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
  //
  // `multiTab: false` opts out of the broker entirely so the engine
  // acts as the sole leader from construction. Without this the tests
  // would skip the leader-gated reconcile path because start() (which
  // is what flips the leader bit in normal use) is never called here.
  return new SyncEngine({
    baseUrl: "http://stub.invalid",
    persist: false,
    reconcileMinIntervalMs: 0,
    multiTab: false,
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
      multiTab: false,
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

  test("a stale 404 cannot remove rows loaded after reset", async () => {
    let respond!: (value: { status: number; body: unknown }) => void;
    let started!: () => void;
    const ready = new Promise<void>((resolve) => { started = resolve; });
    restore = installFetch(async () => {
      started();
      return new Promise((resolve) => { respond = resolve; });
    });
    const engine = makeEngine();
    seedStore(engine, "Doc", [{ id: "same", title: "old" }]);
    const pending = engine.reconcile(["Doc"]);
    await ready;
    await (engine as unknown as { resetReplicaInner(): Promise<void> }).resetReplicaInner();
    seedStore(engine, "Doc", [{ id: "same", title: "new" }]);
    respond({ status: 404, body: {} });
    await pending;
    expect(engine.store.get("Doc", "same")?.title).toBe("new");
  });

  test("transient 403 keeps rows; two consecutive 403s drop them (#343)", async () => {
    // A single 403 can be a bearer caught mid-refresh or a momentary policy
    // blip; nuking the cache on the first one made rows flash away and return
    // next reconcile. The first 403 must keep the rows.
    const status = 403;
    restore = installFetch(async () => ({ status, body: {} }));
    const engine = makeEngine();
    seedStore(engine, "Recording", [{ id: "r1", title: "alive" }]);

    await engine.reconcile(["Recording"]);
    expect(engine.store.list("Recording").length).toBe(1);

    // A second consecutive 403 confirms access is really gone — now drop.
    await engine.reconcile(["Recording"]);
    expect(engine.store.list("Recording").length).toBe(0);
  });

  test("truncated fetch (entity past the 20k cap) upserts but never deletes", async () => {
    // The server has more rows than the 200-page (20k) reconcile cap.
    // fetchEntityRows stops at the cap with `has_more` STILL true, so its
    // result is INCOMPLETE. Pre-fix, reconcile treated every local row
    // absent from that partial set as deleted → it silently tombstoned
    // every row past the cap (then flapped: a full resync re-added them and
    // the next reconcile deleted them again). The fix: a truncated fetch
    // upserts only, never removes.
    let pages = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Event/cursor")) {
        pages += 1;
        // Always claim more remains → the engine exhausts its page cap.
        return {
          status: 200,
          body: {
            data: [{ id: `srv_${pages}`, title: `page ${pages}` }],
            next_cursor: `cursor_${pages}`,
            has_more: true,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    // Local rows that live PAST the cap — the truncated fetch never reaches
    // them. They must survive reconcile.
    seedStore(engine, "Event", [
      { id: "beyond_1", title: "row past the cap" },
      { id: "beyond_2", title: "row past the cap" },
    ]);
    expect(engine.store.list("Event").length).toBe(2);

    await engine.reconcile(["Event"]);

    // The fetch hit the 20-page safety cap (20 × 1000/page = the same 20k
    // row ceiling) — proof the set was truncated.
    expect(pages).toBe(20);
    // The un-fetched local rows are NOT deleted (the bug would drop them).
    expect(engine.store.get("Event", "beyond_1")).not.toBeNull();
    expect(engine.store.get("Event", "beyond_2")).not.toBeNull();
    // Rows the fetch DID return are still upserted — reconcile keeps working.
    expect(engine.store.get("Event", "srv_1")).not.toBeNull();
  });

  test("a successful fetch resets the 403 streak (#343)", async () => {
    let status = 403;
    restore = installFetch(async () =>
      status === 200
        ? {
            status: 200,
            body: {
              data: [{ id: "r1", title: "alive" }],
              next_cursor: null,
              has_more: false,
            },
          }
        : { status: 403, body: {} },
    );
    const engine = makeEngine();
    seedStore(engine, "Recording", [{ id: "r1", title: "alive" }]);

    await engine.reconcile(["Recording"]); // 403 #1 — kept
    expect(engine.store.list("Recording").length).toBe(1);

    status = 200;
    await engine.reconcile(["Recording"]); // success — streak resets
    expect(engine.store.list("Recording").length).toBe(1);

    status = 403;
    await engine.reconcile(["Recording"]); // a fresh lone 403 must NOT drop
    expect(engine.store.list("Recording").length).toBe(1);
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

describe("SyncEngine.reconcile session guard", () => {
  // Regression for the "dashboard flashes data away on first load" bug.
  // The engine starts before the app calls /api/auth/select-org, runs
  // its initial pull + reconcile under tenant=null, and the
  // policy-filtered entity fetch returns 0 rows — which then tombstones
  // every IndexedDB-hydrated row from the previous session. Once the
  // app calls selectOrg the rows reappear, but the user has already
  // seen them flash.
  //
  // The guard: if the resolved session changes mid-fetch (token, tenant,
  // user, isAdmin, or roles), reconcile MUST discard the result. The
  // session-changed envelope queues another reconcile under the new
  // identity.
  test("skips apply when tenant flips during entity fetch", async () => {
    let restore: (() => void) | null = null;
    try {
      const engine = makeEngine();
      // Seed the resolver with a tenant=null session — every call to
      // session.signature() through the rest of this test reflects
      // this value until the fetch handler flips it below.
      engine.session.observeSession({
        userId: "u1",
        tenantId: null,
        isAdmin: false,
        roles: [],
      });

      restore = installFetch(async (url) => {
        if (url.includes("/api/entities/Recording/cursor")) {
          // Flip the session signature WHILE the fetch is "in flight"
          // — between the engine's `sessionBeforeFetch` capture and
          // the apply pass. Models a WS session-changed envelope
          // landing in the gap.
          engine.session.observeSession({
            userId: "u1",
            tenantId: "org-42",
            isAdmin: false,
            roles: [],
          });
          return {
            status: 200,
            body: { data: [], next_cursor: null, has_more: false },
          };
        }
        return { status: 404, body: {} };
      });

      seedStore(engine, "Recording", [{ id: "r1", title: "alive" }]);
      expect(engine.store.list("Recording").length).toBe(1);

      await engine.reconcile(["Recording"]);

      // Row must survive — the stale fetch result was discarded by
      // the session-signature guard.
      expect(engine.store.list("Recording").length).toBe(1);
      expect(engine.store.get("Recording", "r1")).not.toBeNull();
    } finally {
      restore?.();
    }
  });
});

describe("SyncEngine replica identity guard", () => {
  // A different user signing in on the same browser across a page RELOAD
  // must not inherit the previous user's rows. The live `observeToken`
  // reset can't catch this — a fresh engine has no prior token to compare
  // against — so the engine tags the persisted replica with its owner and
  // wipes on a cold-start mismatch. (Privacy leak + "my data vanished"
  // class found in the ai-chat / ai-studio templates.)
  function resolveAs(engine: SyncEngine, userId: string | null): void {
    engine.session.observeSession({
      userId,
      tenantId: null,
      isAdmin: false,
      roles: [],
    });
  }

  test("wipes the replica when a DIFFERENT user is resolved on reload", async () => {
    const engine = makeEngine();
    seedStore(engine, "Note", [{ id: "n1", title: "A's private note" }]);
    (engine as unknown as { _hadCachedReplica: boolean })._hadCachedReplica = true;
    (engine as unknown as { _replicaIdentity: string })._replicaIdentity = "user_A";
    resolveAs(engine, "user_B");

    await (engine as unknown as { guardReplicaIdentity(): Promise<void> }).guardReplicaIdentity();

    expect(engine.store.list("Note").length).toBe(0);
  });

  test("keeps the replica when the SAME user is resolved", async () => {
    const engine = makeEngine();
    seedStore(engine, "Note", [{ id: "n1", title: "mine" }]);
    (engine as unknown as { _hadCachedReplica: boolean })._hadCachedReplica = true;
    (engine as unknown as { _replicaIdentity: string })._replicaIdentity = "user_A";
    resolveAs(engine, "user_A");

    await (engine as unknown as { guardReplicaIdentity(): Promise<void> }).guardReplicaIdentity();

    expect(engine.store.list("Note").length).toBe(1);
  });

  test("keeps an UNTAGGED (pre-fix) replica — conservative, no mass re-download", async () => {
    const engine = makeEngine();
    seedStore(engine, "Note", [{ id: "n1", title: "legacy" }]);
    (engine as unknown as { _hadCachedReplica: boolean })._hadCachedReplica = true;
    // undefined = a replica written before this fix shipped; can't prove a
    // mismatch, so don't wipe (the tag gets written this run for next time).
    (engine as unknown as { _replicaIdentity: undefined })._replicaIdentity = undefined;
    resolveAs(engine, "user_B");

    await (engine as unknown as { guardReplicaIdentity(): Promise<void> }).guardReplicaIdentity();

    expect(engine.store.list("Note").length).toBe(1);
  });

  // The guard wipes rows but keeps queued writes: each write carries its
  // owner, and push sends a guest's writes after the guest signs in and
  // discards another user's (mutation-owner.test.ts).
  test("an identity change across a reload keeps queued writes for the owner check", async () => {
    const cases: ReadonlyArray<readonly [string, string, boolean]> = [
      ["guest_123", "user_B", false],
      ["user_A", "user_B", false],
    ];
    for (const [prev, now, expectWipeMutations] of cases) {
      const engine = makeEngine();
      seedStore(engine, "Note", [{ id: "n1" }]);
      (engine as unknown as { _hadCachedReplica: boolean })._hadCachedReplica = true;
      (engine as unknown as { _replicaIdentity: string })._replicaIdentity = prev;
      resolveAs(engine, now);

      let captured: { wipeMutations?: boolean } | undefined;
      const orig = engine.resetReplica.bind(engine);
      engine.resetReplica = ((opts: { wipeMutations?: boolean } = {}) => {
        captured = opts;
        return orig(opts);
      }) as typeof engine.resetReplica;

      await (engine as unknown as { guardReplicaIdentity(): Promise<void> }).guardReplicaIdentity();

      expect(captured?.wipeMutations).toBe(expectWipeMutations);
    }
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

describe("SyncEngine cookie auth", () => {
  // Regression test for Repro C (v0.3.131): the SyncEngine's `request`
  // method must send `credentials: "include"` on every HTTP call so
  // cookie-authenticated browser sessions reach the server with their
  // session cookie. Without it, /api/sync/pull and the entity
  // reconcile endpoint go anonymous, the default-deny policy returns
  // nothing, and the local replica stays empty forever — even when
  // the same browser session can read every row via the entity API.
  test("pull request sends cookies (credentials: include)", async () => {
    let capturedInit: RequestInit | undefined;
    const restore = installFetch(async (url, init) => {
      capturedInit = init;
      if (url.includes("/api/sync/pull")) {
        return {
          status: 200,
          body: { changes: [], cursor: { last_seq: 0 }, has_more: false },
        };
      }
      return { status: 404, body: {} };
    });
    try {
      const engine = makeEngine();
      await engine.pull();
      expect(capturedInit).toBeDefined();
      expect(capturedInit!.credentials).toBe("include");
    } finally {
      restore();
    }
  });

  test("reconcile entity fetch sends cookies (credentials: include)", async () => {
    let capturedInit: RequestInit | undefined;
    const restore = installFetch(async (url, init) => {
      if (url.includes("/api/entities/")) {
        capturedInit = init;
      }
      return {
        status: 200,
        body: {
          data: [{ id: "r1", title: "alive" }],
          next_cursor: null,
          has_more: false,
        },
      };
    });
    try {
      const engine = makeEngine();
      seedStore(engine, "Recording", [{ id: "r1", title: "alive" }]);
      await engine.reconcile(["Recording"]);
      expect(capturedInit).toBeDefined();
      expect(capturedInit!.credentials).toBe("include");
    } finally {
      restore();
    }
  });

  // field.json() rows carry nested objects/arrays. rowsDiffer must
  // recurse into them (drift at depth is real drift) while staying
  // key-order-insensitive (same content, different key order, is NOT
  // drift — an order-sensitive compare would re-upsert every json row
  // on every reconcile pass).
  test("nested json drift updates; key-order difference does not thrash", async () => {
    const restoreFetch = installFetch(async (url) => {
      if (url.includes("/api/entities/Doc/cursor")) {
        return {
          status: 200,
          body: {
            data: [
              // drifted at depth vs the seeded row
              { id: "doc_1", config: { theme: "dark", panes: [{ w: 2 }] } },
              // identical content to the seed, different key order
              { id: "doc_2", config: { b: 2, a: 1 } },
            ],
            next_cursor: null,
            has_more: false,
          },
        };
      }
      return { status: 404, body: {} };
    });

    const engine = makeEngine();
    seedStore(engine, "Doc", [
      { id: "doc_1", config: { theme: "light", panes: [{ w: 2 }] } },
      { id: "doc_2", config: { a: 1, b: 2 } },
    ]);

    let notifications = 0;
    const unsub = engine.store.subscribe(() => {
      notifications += 1;
    });
    try {
      await engine.reconcile(["Doc"]);
    } finally {
      unsub();
      restoreFetch();
    }

    const drifted = engine.store.get("Doc", "doc_1") as {
      config: { theme: string };
    } | null;
    expect(drifted!.config.theme).toBe("dark");
    const stable = engine.store.get("Doc", "doc_2") as {
      config: Record<string, number>;
    } | null;
    expect(stable!.config).toEqual({ a: 1, b: 2 });
    // Exactly one row changed → exactly one store notification.
    expect(notifications).toBe(1);
  });
});

// The server sends `sync-reconcile` when a row of another entity changed
// which rows of these entities the user may read (a share, an unshare):
// no change event of the rows themselves will come.
describe("sync-reconcile hint", () => {
  let restore: (() => void) | null = null;

  afterEach(() => {
    restore?.();
    restore = null;
  });

  type Internals = {
    dispatchInboundMessage(m: Record<string, unknown>): void;
    running: boolean;
    cursor: { last_seq: number };
    pendingReconcile: Map<string, number>;
  };
  const inner = (engine: SyncEngine) => engine as unknown as Internals;
  const deliver = (engine: SyncEngine, msg: Record<string, unknown>) => inner(engine).dispatchInboundMessage(msg);
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  function documentServer(onFetch: (n: number) => void = () => {}) {
    let fetches = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Document/cursor")) {
        fetches += 1;
        onFetch(fetches);
        return {
          status: 200,
          body: { data: [{ id: "shared", title: "Now shared" }], next_cursor: null, has_more: false },
        };
      }
      return { status: 404, body: {} };
    });
    return () => fetches;
  }

  test("reconciles the named entities: the shared row arrives, the unshared one leaves", async () => {
    const fetches = documentServer();
    const engine = makeEngine();
    inner(engine).running = true;
    seedStore(engine, "Document", [{ id: "unshared", title: "No longer shared" }]);

    deliver(engine, { type: "sync-reconcile", entities: ["Document"] });
    await sleep(250);

    expect(fetches()).toBe(1);
    expect(engine.store.list("Document").map((r) => (r as { id: string }).id)).toEqual(["shared"]);
    engine.stop();
  });

  test("hints that arrive together share one fetch", async () => {
    const fetches = documentServer();
    const engine = makeEngine();
    inner(engine).running = true;
    for (let i = 0; i < 5; i++) deliver(engine, { type: "sync-reconcile", entities: ["Document"] });
    await sleep(250);
    expect(fetches()).toBe(1);
    engine.stop();
  });

  test("keeps retrying while newer changes race the fetch, then lands", async () => {
    const engine = makeEngine();
    // The first two fetches race a change event: the cursor moves.
    const fetches = documentServer((n) => {
      if (n <= 2) inner(engine).cursor.last_seq += 1;
    });
    inner(engine).running = true;

    deliver(engine, { type: "sync-reconcile", entities: ["Document"] });
    await sleep(250);
    expect(fetches()).toBe(1);
    expect(engine.store.list("Document")).toEqual([]);
    expect([...inner(engine).pendingReconcile.keys()]).toEqual(["Document"]);

    // Retries run at most once a second.
    await sleep(2300);
    expect(fetches()).toBe(3);
    expect(engine.store.list("Document").map((r) => (r as { id: string }).id)).toEqual(["shared"]);
    expect(inner(engine).pendingReconcile.size).toBe(0);
    engine.stop();
  });

  test("stop drops pending hints", async () => {
    const fetches = documentServer();
    const engine = makeEngine();
    inner(engine).running = true;
    deliver(engine, { type: "sync-reconcile", entities: ["Document"] });
    engine.stop();
    await sleep(250);
    expect(fetches()).toBe(0);
    expect(inner(engine).pendingReconcile.size).toBe(0);
  });

  test("ignores a hint without entity names", async () => {
    const fetches = documentServer();
    const engine = makeEngine();
    inner(engine).running = true;
    deliver(engine, { type: "sync-reconcile", entities: [42, null] });
    deliver(engine, { type: "sync-reconcile" });
    await sleep(250);
    expect(fetches()).toBe(0);
    engine.stop();
  });
});

describe("sync-reconcile hint after a failed fetch", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("a 5xx keeps the hint pending and the retry lands", async () => {
    let fetches = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Document/cursor")) {
        fetches += 1;
        if (fetches === 1) return { status: 503, body: {} };
        return { status: 200, body: { data: [], next_cursor: null, has_more: false } };
      }
      return { status: 404, body: {} };
    });
    const engine = makeEngine();
    const inner = engine as unknown as {
      running: boolean;
      pendingReconcile: Map<string, number>;
      dispatchInboundMessage(m: Record<string, unknown>): void;
    };
    inner.running = true;
    seedStore(engine, "Document", [{ id: "unshared", title: "No longer shared" }]);

    inner.dispatchInboundMessage({ type: "sync-reconcile", entities: ["Document"] });
    await new Promise((r) => setTimeout(r, 250));
    expect(fetches).toBe(1);
    expect([...inner.pendingReconcile.keys()]).toEqual(["Document"]);
    expect(engine.store.list("Document").length).toBe(1);

    await new Promise((r) => setTimeout(r, 1200));
    expect(fetches).toBe(2);
    expect(engine.store.list("Document")).toEqual([]);
    engine.stop();
  });
});

// Any reconcile, not only a hinted one: an entity skipped because its fetch
// raced a newer change is retried, so a row that became readable while the
// page was away (a new folder, a share) still arrives.
describe("a skipped reconcile retries", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test("a startup reconcile that raced a change lands on the retry", async () => {
    const engine = makeEngine();
    const inner = engine as unknown as { running: boolean; cursor: { last_seq: number }; pendingReconcile: Set<string> };
    let fetches = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Folder/cursor")) {
        fetches += 1;
        if (fetches === 1) inner.cursor.last_seq += 1;
        return { status: 200, body: { data: [{ id: "f1", name: "Team" }], next_cursor: null, has_more: false } };
      }
      return { status: 404, body: {} };
    });
    inner.running = true;

    await engine.reconcile(["Folder"]);
    expect(engine.store.list("Folder")).toEqual([]);
    expect([...inner.pendingReconcile.keys()]).toEqual(["Folder"]);

    await new Promise((r) => setTimeout(r, 1300));
    expect(fetches).toBe(2);
    expect(engine.store.list("Folder").map((r) => (r as { id: string }).id)).toEqual(["f1"]);
    engine.stop();
  });
});

describe("pending reconciles end when they should", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  type Internals = {
    running: boolean;
    pendingReconcile: Map<string, number>;
    opQueue: { enqueue<T>(key: string, fn: () => Promise<T>): Promise<T> };
    dispatchInboundMessage(m: Record<string, unknown>): void;
  };
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  function server(status: () => number) {
    let fetches = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Document/cursor")) {
        fetches += 1;
        const s = status();
        if (s !== 200) return { status: s, body: {} };
        return { status: 200, body: { data: [], next_cursor: null, has_more: false } };
      }
      return { status: 404, body: {} };
    });
    return () => fetches;
  }

  test("a later successful reconcile clears the pending retry", async () => {
    let failing = true;
    const fetches = server(() => (failing ? 503 : 200));
    const engine = makeEngine();
    const inner = engine as unknown as Internals;
    inner.running = true;
    await engine.reconcile(["Document"]);
    expect([...inner.pendingReconcile.keys()]).toEqual(["Document"]);
    failing = false;
    await engine.reconcile(["Document"]);
    expect(inner.pendingReconcile.size).toBe(0);
    await sleep(1300);
    expect(fetches()).toBe(2);
    engine.stop();
  });

  test("stop cancels a retry already waiting in the queue", async () => {
    const fetches = server(() => 200);
    const engine = makeEngine();
    const inner = engine as unknown as Internals;
    inner.running = true;
    // A slow operation holds the queue while the retry's timer fires.
    void inner.opQueue.enqueue("slow", () => sleep(400));
    inner.dispatchInboundMessage({ type: "sync-reconcile", entities: ["Document"] });
    await sleep(200);
    engine.stop();
    await sleep(400);
    expect(fetches()).toBe(0);
  });

  test("a replica reset drops pending retries", async () => {
    const fetches = server(() => 503);
    const engine = makeEngine();
    const inner = engine as unknown as Internals;
    inner.running = true;
    await engine.reconcile(["Document"]);
    expect(fetches()).toBe(1);
    await engine.resetReplica();
    expect(inner.pendingReconcile.size).toBe(0);
    await sleep(1300);
    expect(fetches()).toBe(1);
    engine.stop();
  });
});

describe("pending reconciles and newer requests", () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  type Internals = {
    running: boolean;
    pendingReconcile: Map<string, number>;
    dispatchInboundMessage(m: Record<string, unknown>): void;
  };

  test("a hint that arrives during a fetch survives that fetch", async () => {
    const engine = makeEngine();
    const inner = engine as unknown as Internals;
    let fetches = 0;
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Document/cursor")) {
        fetches += 1;
        // The server already built this response when the share lands.
        if (fetches === 1) inner.dispatchInboundMessage({ type: "sync-reconcile", entities: ["Document"] });
        return { status: 200, body: { data: [], next_cursor: null, has_more: false } };
      }
      return { status: 404, body: {} };
    });
    inner.running = true;
    await engine.reconcile(["Document"]);
    expect([...inner.pendingReconcile.keys()]).toEqual(["Document"]);
    await sleep(1300);
    expect(fetches).toBe(2);
    engine.stop();
  });

  test("a reconcile in flight during a reset does not bring retries back", async () => {
    const engine = makeEngine();
    const inner = engine as unknown as Internals;
    let fetches = 0;
    let release: () => void = () => {};
    restore = installFetch(async (url) => {
      if (url.includes("/api/entities/Document/cursor")) {
        fetches += 1;
        if (fetches === 1) await new Promise<void>((r) => (release = r));
        return { status: 503, body: {} };
      }
      return { status: 404, body: {} };
    });
    inner.running = true;
    const pass = engine.reconcile(["Document"]);
    await sleep(20);
    const reset = engine.resetReplica();
    release();
    await pass;
    await reset;
    expect(inner.pendingReconcile.size).toBe(0);
    await sleep(1300);
    expect(fetches).toBe(1);
    engine.stop();
  });
});

test("two callers sharing one full reconcile keep a hint that arrived between them", async () => {
  const engine = makeEngine();
  const inner = engine as unknown as {
    running: boolean;
    pendingReconcile: Map<string, number>;
    store: { applyChange(ev: ChangeEvent): void };
    dispatchInboundMessage(m: Record<string, unknown>): void;
  };
  seedStore(engine, "Document", [{ id: "d0", title: "seed" }]);
  let fetches = 0;
  let release: () => void = () => {};
  const restore = installFetch(async (url) => {
    if (url.includes("/api/entities/Document/cursor")) {
      fetches += 1;
      if (fetches === 1) await new Promise<void>((r) => (release = r));
      return { status: 200, body: { data: [{ id: "d0", title: "seed" }], next_cursor: null, has_more: false } };
    }
    return { status: 404, body: {} };
  });
  try {
    inner.running = true;
    const first = engine.reconcile();
    await new Promise((r) => setTimeout(r, 20));
    inner.dispatchInboundMessage({ type: "sync-reconcile", entities: ["Document"] });
    const second = engine.reconcile();
    release();
    await Promise.all([first, second]);
    expect([...inner.pendingReconcile.keys()]).toEqual(["Document"]);
  } finally {
    engine.stop();
    restore();
  }
});
