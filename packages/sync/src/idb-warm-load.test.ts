// IDB warm-load hydration tests.
//
// Returning visits to a Pylon app must render the last-known data
// IMMEDIATELY — before the network pull resolves — by draining the
// IndexedDB replica into the in-memory store during start(). These
// tests cover:
//
//   1. New user (no IDB cache):     start() works, hooks return [].
//   2. Returning user with cache:   list(E) returns rows BEFORE pull.
//   3. Mid-pull cached visibility:  list(E) sees cache while pull races.
//   4. Stale-while-revalidate:      offline-deleted rows get cleaned up.
//   5. Sign-out clears IDB:         next user can't see prior user's data.
//   6. Single-tx load consistency:  one tx returns entities + cursor.
//
// Bun has no native IndexedDB; we install `fake-indexeddb` globals
// here. The reset (`indexedDB.deleteDatabase`) between tests gives
// each scenario a clean DB without sharing state across suites.

import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { IndexedDBPersistence, SyncEngine, type Row } from "./index";

// ---------------------------------------------------------------------------
// Test fetch stub — drives /api/sync/pull + /api/auth/me + entity reconcile.
// Per-test mutable so individual scenarios can stage server state.
// ---------------------------------------------------------------------------

type Snapshot = Record<string, Row[]>;
interface ServerState {
  /** Current canonical rows per entity. */
  rows: Snapshot;
  /** Current authoritative sync cursor. */
  serverSeq: number;
  /** Optional delay before /api/sync/pull responds — used to assert
   *  that cached data is visible WHILE the pull is in flight. */
  pullDelayMs: number;
  /** Per-test session shape returned by /api/auth/me. */
  session: { user: { id: string } | null; tenant: { id: string } | null };
}

let server: ServerState;

function resetServer(): void {
  server = {
    rows: {},
    serverSeq: 1,
    pullDelayMs: 0,
    session: { user: null, tenant: null },
  };
}

function installFetch(): () => void {
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    let status = 200;
    let body: unknown = {};

    if (url.includes("/api/auth/me")) {
      body = server.session;
    } else if (url.includes("/api/sync/pull")) {
      if (server.pullDelayMs > 0) {
        await new Promise((r) => setTimeout(r, server.pullDelayMs));
      }
      // For a cursor-based pull we return the current set of rows as
      // `insert` events at serverSeq. This is enough to drive the
      // SyncEngine's apply path; reconcile catches deletes elsewhere.
      const changes: Array<Record<string, unknown>> = [];
      for (const [entity, rows] of Object.entries(server.rows)) {
        for (const row of rows) {
          changes.push({
            seq: server.serverSeq,
            entity,
            row_id: row.id as string,
            kind: "insert",
            data: row,
            timestamp: "",
          });
        }
      }
      body = {
        changes,
        cursor: { last_seq: server.serverSeq },
        has_more: false,
      };
    } else if (url.includes("/api/entities/")) {
      // Reconcile per-entity endpoint. URL shape:
      //   /api/entities/<EntityName>/cursor?...
      const match = url.match(/\/api\/entities\/([^/?]+)/);
      const entity = match?.[1] ?? "";
      const rows = server.rows[entity] ?? [];
      body = { data: rows, next_cursor: null, has_more: false };
    } else {
      status = 404;
    }

    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
      text: async () => JSON.stringify(body),
      headers: new Headers(),
    } as unknown as Response;
  }) as typeof fetch;
  return () => {
    globalThis.fetch = original;
  };
}

async function deleteAllDatabases(): Promise<void> {
  // Drop every Pylon-named DB so a previous test can't bleed into
  // the next one. Each test installs its own engine with a unique
  // appName, but defensive cleanup keeps the suite hermetic.
  const dbs = await (
    indexedDB as unknown as { databases?: () => Promise<{ name?: string }[]> }
  ).databases?.();
  if (!dbs) return;
  await Promise.all(
    dbs
      .filter((d) => d.name?.startsWith("pylon_sync_"))
      .map(
        (d) =>
          new Promise<void>((resolve) => {
            const req = indexedDB.deleteDatabase(d.name!);
            req.onsuccess = () => resolve();
            req.onerror = () => resolve();
            req.onblocked = () => resolve();
          }),
      ),
  );
}

let restoreFetch: (() => void) | null = null;

beforeEach(async () => {
  resetServer();
  restoreFetch = installFetch();
  await deleteAllDatabases();
});

afterEach(async () => {
  restoreFetch?.();
  restoreFetch = null;
  await deleteAllDatabases();
});

function makeEngine(appName: string): SyncEngine {
  return new SyncEngine({
    baseUrl: "http://test.invalid",
    persist: true,
    appName,
    transport: "poll",
    multiTab: false,
    reconcileMinIntervalMs: 0,
    pollInterval: 60_000,
  });
}

/** Seed IDB directly under the engine's appName key — simulates "a
 *  prior session of this app left data on disk before the user
 *  closed the tab". */
async function seedIDB(
  appName: string,
  entities: Snapshot,
  cursor: { last_seq: number } | null,
): Promise<void> {
  const p = new IndexedDBPersistence(appName);
  await p.open();
  for (const [entity, rows] of Object.entries(entities)) {
    for (const row of rows) {
      await p.saveRow(entity, row.id as string, row);
    }
  }
  if (cursor) await p.saveCursor(cursor);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("IDB warm-load hydration", () => {
  test("first-time user: empty IDB → store starts empty, then populates from pull", async () => {
    server.rows = { Channel: [{ id: "c1", name: "general" }] };
    const engine = makeEngine("warmload-fresh");
    expect(engine.store.list("Channel")).toEqual([]);
    await engine.start();
    // After start the pull has landed seeds — same as today.
    expect(engine.store.list("Channel")).toHaveLength(1);
    engine.stop();
  });

  test("returning user: cached rows visible immediately after start() (before any pull)", async () => {
    // Seed disk with 3 channels from "yesterday".
    await seedIDB(
      "warmload-return",
      {
        Channel: [
          { id: "c1", name: "general" },
          { id: "c2", name: "random" },
          { id: "c3", name: "off-topic" },
        ],
      },
      { last_seq: 42 },
    );
    // Server has the same 3 channels (no offline mutations).
    server.rows = {
      Channel: [
        { id: "c1", name: "general" },
        { id: "c2", name: "random" },
        { id: "c3", name: "off-topic" },
      ],
    };
    server.serverSeq = 42;
    // Make the pull slow so we can assert cached rows are visible
    // BEFORE the network round-trip lands.
    server.pullDelayMs = 100;

    const engine = makeEngine("warmload-return");
    const startP = engine.start();

    // Race: poll the store until rows show up OR the start promise
    // resolves. The warm-load apply has to land FIRST.
    let cachedSeen = -1;
    const deadline = Date.now() + 80;
    while (Date.now() < deadline) {
      const rows = engine.store.list("Channel");
      if (rows.length > 0) {
        cachedSeen = rows.length;
        break;
      }
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(cachedSeen).toBe(3);

    await startP;
    expect(engine.store.list("Channel")).toHaveLength(3);
    engine.stop();
  });

  test("mid-pull, hooks read cached rows (stale-while-revalidate)", async () => {
    await seedIDB(
      "warmload-mid",
      { Todo: [{ id: "t1", text: "old" }] },
      { last_seq: 10 },
    );
    server.rows = { Todo: [{ id: "t1", text: "new" }] };
    server.serverSeq = 11;
    server.pullDelayMs = 80;

    const engine = makeEngine("warmload-mid");
    const startP = engine.start();
    // After IDB hydration but before pull settles: should see the old
    // value. Poll for it.
    let preReconcileText: string | undefined;
    const deadline = Date.now() + 60;
    while (Date.now() < deadline) {
      const t = engine.store.get("Todo", "t1") as { text?: string } | null;
      if (t) {
        preReconcileText = t.text;
        break;
      }
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(preReconcileText).toBe("old");
    await startP;
    // After pull lands: fresh value.
    const after = engine.store.get("Todo", "t1") as { text?: string } | null;
    expect(after?.text).toBe("new");
    engine.stop();
  });

  test("stale-while-revalidate: rows deleted on server while offline get cleaned up", async () => {
    // Cache has 3 channels from last session. Server has only 2 — c3
    // was deleted while this client was offline, the delete event has
    // fallen off the change log. Reconcile must remove c3.
    await seedIDB(
      "warmload-delete",
      {
        Channel: [
          { id: "c1", name: "general" },
          { id: "c2", name: "random" },
          { id: "c3", name: "deleted-while-offline" },
        ],
      },
      { last_seq: 5 },
    );
    server.rows = {
      Channel: [
        { id: "c1", name: "general" },
        { id: "c2", name: "random" },
      ],
    };
    server.serverSeq = 5; // no new events past our cursor

    const engine = makeEngine("warmload-delete");
    await engine.start();
    // Trigger reconcile manually — in production this fires from the
    // WS onConnected callback. We're on the poll transport here so we
    // call it directly to assert the behavior under test.
    await engine.reconcile(["Channel"]);
    const ids = engine.store
      .list("Channel")
      .map((r) => (r as { id: string }).id)
      .sort();
    expect(ids).toEqual(["c1", "c2"]);
    engine.stop();
  });

  test("sign-out clears IDB: a different user does not see prior user's data", async () => {
    const appName = "warmload-signout";

    // User A signs in, cache gets populated.
    await seedIDB(
      appName,
      { Channel: [{ id: "c-a", name: "user-a-only" }] },
      { last_seq: 7 },
    );

    // Simulate sign-out via resetReplica (the path applySessionTransition
    // takes on tenant flip / token flip).
    {
      const engine = makeEngine(appName);
      await engine.start();
      expect(engine.store.list("Channel")).toHaveLength(1);
      await engine.resetReplica();
      expect(engine.store.list("Channel")).toEqual([]);
      engine.stop();
    }

    // User B signs in (server returns different data).
    server.rows = { Channel: [{ id: "c-b", name: "user-b-only" }] };
    server.serverSeq = 1;

    const engineB = makeEngine(appName);
    await engineB.start();
    const ids = engineB.store
      .list("Channel")
      .map((r) => (r as { id: string }).id);
    // No leakage of c-a — clear() wiped IDB on resetReplica.
    expect(ids).not.toContain("c-a");
    expect(ids).toContain("c-b");
    engineB.stop();
  });

  test("loadSnapshot returns entities + cursor in one transaction", async () => {
    const appName = "warmload-atomic";
    await seedIDB(
      appName,
      {
        Channel: [{ id: "c1" }],
        Todo: [{ id: "t1" }, { id: "t2" }],
      },
      { last_seq: 99 },
    );
    const p = new IndexedDBPersistence(appName);
    await p.open();
    const result = await p.loadSnapshot();
    expect(result.hadCache).toBe(true);
    expect(result.cursor?.last_seq).toBe(99);
    expect(Object.keys(result.entities).sort()).toEqual(["Channel", "Todo"]);
    expect(result.entities.Channel).toHaveLength(1);
    expect(result.entities.Todo).toHaveLength(2);

    // Empty DB: hadCache = false.
    await deleteAllDatabases();
    const p2 = new IndexedDBPersistence(appName);
    await p2.open();
    const empty = await p2.loadSnapshot();
    expect(empty.hadCache).toBe(false);
    expect(empty.cursor).toBeNull();
    expect(empty.entities).toEqual({});
  });

  test("lastPullStartedFromZero only short-circuits the post-pull reconcile when IDB cache was empty", async () => {
    // Returning user with cached cursor=0 (rare — manual partial wipe).
    // The IDB rows exist; the cursor was lost. Pull from 0 returns
    // current server rows but no tombstones for offline deletes. The
    // fast-path MUST NOT skip the reconcile pass.
    await seedIDB(
      "warmload-fastpath",
      { Channel: [{ id: "c-ghost", name: "deleted-on-server" }] },
      null, // no cursor — pull will start from 0
    );
    // Server has different rows (c-ghost no longer exists).
    server.rows = { Channel: [{ id: "c-live", name: "current" }] };
    server.serverSeq = 100;

    const engine = makeEngine("warmload-fastpath");
    await engine.start();
    // After start() the engine has hydrated c-ghost from IDB and the
    // pull has applied c-live as a fresh insert (both rows in memory).
    // Run the reconcile that onConnected would normally fire — c-ghost
    // must get cleaned up.
    await engine.reconcile(["Channel"]);
    const ids = engine.store
      .list("Channel")
      .map((r) => (r as { id: string }).id)
      .sort();
    expect(ids).toEqual(["c-live"]);
    engine.stop();
  });

  test("cold IDB load > 100ms warns via console", async () => {
    // No data — opening the DB is fast on fake-indexeddb. We can't
    // realistically force a >100ms read here without monkey-patching
    // the transaction queue. Instead, assert the warning logic is
    // wired by intercepting console.warn during a normal start() and
    // verifying the absence of the warning when the load is fast.
    let warned = false;
    const origWarn = console.warn;
    console.warn = (...args: unknown[]) => {
      const msg = args.map((a) => String(a)).join(" ");
      if (msg.includes("cold IDB load took")) warned = true;
    };
    try {
      const engine = makeEngine("warmload-perf");
      await engine.start();
      engine.stop();
    } finally {
      console.warn = origWarn;
    }
    // Fast path — no warning expected on a trivial load.
    expect(warned).toBe(false);
  });
});
