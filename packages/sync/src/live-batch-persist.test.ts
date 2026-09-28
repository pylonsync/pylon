// Live change frames are group-committed to IndexedDB.
//
// Each WebSocket change frame used to be its own apply step: one store
// notify, one IndexedDB transaction for the row, and a second one for the
// cursor, all awaited before the next frame could apply. A busy channel
// capped each tab at a few hundred row changes per second. Frames that
// arrive while a write is in flight now join one batch: one notify and one
// transaction that holds the rows and the cursor.

import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { IndexedDBPersistence, SyncEngine, type ChangeEvent } from "./index";

const realFetch = globalThis.fetch;

beforeEach(() => {
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = String(input);
    const body = url.includes("/api/sync/pull")
      ? { changes: [], cursor: { last_seq: 0 }, has_more: false }
      : url.includes("/api/auth/me")
        ? { user_id: "u1" }
        : {};
    return new Response(JSON.stringify(body), { status: 200 });
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

async function startEngine() {
  const persistence = new IndexedDBPersistence(`live-batch-${Math.random().toString(36).slice(2)}`);
  const engine = new SyncEngine({
    baseUrl: "http://test.invalid",
    persistence,
    multiTab: false,
    transport: "poll",
    pollInterval: 1e9,
    reconcileOnVisibility: false,
  });
  await engine.start();
  const db = persistence.connection!;
  const realTx = db.transaction.bind(db);
  const counts = { writeTx: 0 };
  (db as unknown as { transaction: typeof db.transaction }).transaction = ((
    ...args: Parameters<typeof db.transaction>
  ) => {
    if (String(args[1]) === "readwrite") counts.writeTx += 1;
    return realTx(...args);
  }) as typeof db.transaction;
  let notifies = 0;
  engine.store.subscribe(() => {
    notifies += 1;
  });
  const internals = engine as unknown as {
    transportHost(): { onChangeEvent(ev: ChangeEvent): void };
    applyQueue: Promise<void>;
  };
  const host = internals.transportHost();
  return {
    engine,
    persistence,
    counts,
    notifies: () => notifies,
    /** Deliver frames one task each, like WebSocket messages. */
    async deliver(frames: ChangeEvent[]) {
      const channel = new MessageChannel();
      await new Promise<void>((resolve) => {
        let n = 0;
        channel.port1.onmessage = (m) => {
          host.onChangeEvent(frames[m.data as number]!);
          if (++n === frames.length) resolve();
        };
        frames.forEach((_, i) => channel.port2.postMessage(i));
      });
      channel.port1.close();
      const last = frames[frames.length - 1]!.seq;
      while (engine.getCursor().last_seq < last) {
        await internals.applyQueue;
        await new Promise((r) => setTimeout(r, 0));
      }
      await internals.applyQueue;
    },
  };
}

function insert(seq: number, id: string, data: Record<string, unknown>): ChangeEvent {
  return { seq, entity: "Msg", row_id: id, kind: "insert", data: { id, ...data }, timestamp: "" };
}

describe("live frame group commit", () => {
  test("a burst of frames shares IndexedDB transactions and store notifies", async () => {
    const t = await startEngine();
    const frames = Array.from({ length: 300 }, (_, i) => insert(i + 1, `m${i}`, { body: `hi ${i}` }));
    const notifiesBefore = t.notifies();
    const t0 = performance.now();
    await t.deliver(frames);
    const ms = performance.now() - t0;

    const snap = await t.persistence.loadSnapshot();
    expect(snap.entities.Msg).toHaveLength(300);
    expect(snap.cursor?.last_seq).toBe(300);
    expect(t.engine.store.list("Msg")).toHaveLength(300);
    // One frame per step used to cost two write transactions (row, then
    // cursor): 600 here. Batches cost one each.
    expect(t.counts.writeTx).toBeLessThan(60);
    expect(t.notifies() - notifiesBefore).toBeLessThan(60);
    console.log(
      `[bench] 300 frames: ${Math.round(ms)} ms, ${t.counts.writeTx} write transactions, ` +
        `${t.notifies() - notifiesBefore} notifies (fake-indexeddb)`,
    );
    t.engine.stop();
  });

  test("order inside a batch holds on disk: insert, update, delete, re-insert", async () => {
    const t = await startEngine();
    await t.deliver([
      insert(1, "a", { v: 1 }),
      { seq: 2, entity: "Msg", row_id: "a", kind: "update", data: { v: 2 }, timestamp: "" },
      insert(3, "b", { v: 1 }),
      { seq: 4, entity: "Msg", row_id: "b", kind: "delete", timestamp: "" } as ChangeEvent,
      insert(5, "c", { v: 1 }),
      { seq: 6, entity: "Msg", row_id: "c", kind: "update", data: { v: 3 }, timestamp: "" },
    ]);
    const snap = await t.persistence.loadSnapshot();
    const byId = Object.fromEntries((snap.entities.Msg ?? []).map((r) => [r.id, r]));
    expect(byId.a).toMatchObject({ id: "a", v: 2 });
    expect(byId.b).toBeUndefined();
    expect(byId.c).toMatchObject({ id: "c", v: 3 });
    expect(snap.cursor?.last_seq).toBe(6);
    t.engine.stop();
  });

  test("a stale or repeated frame inside a batch is dropped, as before", async () => {
    const t = await startEngine();
    await t.deliver([
      insert(1, "a", { v: 1 }),
      { seq: 3, entity: "Msg", row_id: "a", kind: "update", data: { v: 3 }, timestamp: "" },
      // Arrives after seq 3 in the same batch: older, must not win.
      { seq: 2, entity: "Msg", row_id: "a", kind: "update", data: { v: 2 }, timestamp: "" },
      { seq: 3, entity: "Msg", row_id: "a", kind: "update", data: { v: 3 }, timestamp: "" },
    ]);
    expect(t.engine.store.get("Msg", "a")).toMatchObject({ v: 3 });
    const snap = await t.persistence.loadSnapshot();
    expect(snap.entities.Msg?.[0]).toMatchObject({ v: 3 });
    t.engine.stop();
  });

  test("an aborted batch leaves the on-disk cursor behind the rows", async () => {
    const t = await startEngine();
    await t.deliver([insert(1, "a", { v: 1 })]);
    const db = t.persistence.connection!;
    const wrapped = db.transaction.bind(db);
    let armed = true;
    (db as unknown as { transaction: typeof db.transaction }).transaction = ((
      ...args: Parameters<typeof db.transaction>
    ) => {
      const tx = wrapped(...args);
      if (armed && String(args[1]) === "readwrite") {
        armed = false;
        queueMicrotask(() => tx.abort());
      }
      return tx;
    }) as typeof db.transaction;
    const warn = console.warn;
    console.warn = () => {};
    try {
      await t.deliver([insert(2, "b", { v: 1 }), insert(3, "c", { v: 1 })]);
    } finally {
      console.warn = warn;
    }
    // Memory moved on; disk kept the cursor at the last committed batch.
    expect(t.engine.getCursor().last_seq).toBe(3);
    const snap = await t.persistence.loadSnapshot();
    expect(snap.cursor?.last_seq).toBe(1);
    t.engine.stop();
  });
});
