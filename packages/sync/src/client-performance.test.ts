import "fake-indexeddb/auto";
import { afterEach, expect, test } from "bun:test";
import { LocalStore } from "./local-store";
import { MutationQueue } from "./mutation-queue";
import { IndexedDBMutationPersistence, IndexedDBPersistence, persistChange } from "./persistence";
import { SyncEngine, type ChangeEvent, type ReplicaPersistence, type Row } from "./index";

const disks: IndexedDBPersistence[] = [];
afterEach(() => { for (const disk of disks.splice(0)) disk.connection?.close(); });

function change(entity: string, id: string): ChangeEvent {
  return { seq: 1, entity, row_id: id, kind: "insert", data: { id }, timestamp: "" };
}

async function disk() {
  const persistence = new IndexedDBPersistence(`client-perf-${crypto.randomUUID()}`);
  await persistence.open();
  disks.push(persistence);
  return persistence;
}

function reconcileEngine(persistence: ReplicaPersistence) {
  const engine = new SyncEngine({ baseUrl: "http://test.invalid", multiTab: false });
  const internals = engine as unknown as {
    persistence: ReplicaPersistence;
    replicaEpoch: number;
    persistDegraded: boolean;
    applyQueue: Promise<void>;
    dropEntity(entity: string, seq: number): Promise<void>;
    enqueueReconcile(entity: string, rows: Row[], removed: string[], seq: number): Promise<void>;
    enqueueApply(changes: ChangeEvent[], cursor: { last_seq: number }): Promise<void>;
  };
  internals.persistence = persistence;
  engine.store._persistFn = (ev) => persistChange(persistence, ev);
  return { engine, internals };
}

test("entity subscriptions run once per relevant batch; global signals reach all", () => {
  const store = new LocalStore();
  const calls = { a: 0, b: 0, c: 0, global: 0 };
  store.subscribe(() => calls.a++, "A");
  store.subscribe(() => calls.b++, "B");
  store.subscribe(() => calls.c++, "C");
  store.subscribe(() => calls.global++);
  const beforeC = store.version("C");
  store.applyInMemory([change("A", "1"), change("B", "2"), change("A", "3")]);
  expect(calls).toEqual({ a: 1, b: 1, c: 0, global: 1 });
  expect(store.version("C")).toBe(beforeC);
  const beforeA = store.version("A");
  store.notify();
  expect(store.version("A")).toBe(beforeA);
  expect(calls).toEqual({ a: 2, b: 2, c: 1, global: 2 });
  store.clearAll();
  expect(store.version("A")).toBeGreaterThan(beforeA);
  expect(store.version("C")).toBeGreaterThan(beforeC);
  expect(calls).toEqual({ a: 3, b: 3, c: 2, global: 3 });
});

test("every row mutation invalidates the entity version", async () => {
  const store = new LocalStore();
  const mutate = (fn: () => void) => {
    const before = store.version("A");
    fn();
    expect(store.version("A")).toBeGreaterThan(before);
  };
  mutate(() => store.applyChange(change("A", "1")));
  mutate(() => store.optimisticUpdate("A", "1", { text: "new" }));
  mutate(() => store.optimisticDelete("A", "1"));
  mutate(() => store.restoreRow("A", "1", { id: "1" }));
  mutate(() => store.reconcileRemove("A", "1", 5));
  mutate(() => store.optimisticInsertWithId("A", "2", {}));
  mutate(() => store.rollbackOptimisticInsert("A", "2"));
  mutate(() => store.optimisticInsertWithId("A", "3", {}));
  mutate(() => store.revokeRow("A", "3", 5));
  mutate(() => store.applyReconcileInMemory("A", [{ id: "4" }], [], 5));
});

test("row versions track writes, removals, reconciliation, and reset", () => {
  const store = new LocalStore();
  store.optimisticInsertWithId("A", "unrelated", {});
  const unrelated = store.rowVersion("A", "unrelated");
  const mutate = (fn: () => void) => {
    const before = store.rowVersion("A", "1");
    fn();
    expect(store.rowVersion("A", "1")).not.toBe(before);
    expect(store.rowVersion("A", "unrelated")).toBe(unrelated);
  };
  mutate(() => store.applyChange(change("A", "1")));
  mutate(() => store.optimisticUpdate("A", "1", { text: "new" }));
  mutate(() => store.optimisticDelete("A", "1"));
  mutate(() => store.restoreRow("A", "1", { id: "1" }));
  mutate(() => store.rollbackOptimisticInsert("A", "1"));
  mutate(() => store.optimisticInsertWithId("A", "1", {}));
  mutate(() => store.reconcileRemove("A", "1", 5));
  mutate(() => store.applyReconcileInMemory("A", [{ id: "1" }], [], 6));
  mutate(() => store.revokeRow("A", "1", 7));
  const missing = store.rowVersion("A", "1");
  store.applyChange(change("A", "1"));
  expect(store.rowVersion("A", "1")).toBe(missing);
  store.clearAll();
  expect(store.rowVersion("A", "1")).not.toBe(missing);
  expect(store.rowVersion("A", "unrelated")).not.toBe(unrelated);
});

test("1000 reconcile rows use one transaction and retain cursor and tombstone fences", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  await persistence.saveCursor({ last_seq: 20 });
  await persistence.saveRow("A", "old", { id: "old" });
  engine.store.applyChange(change("A", "old"));
  engine.store.revokeRow("A", "blocked", 30);
  const db = persistence.connection!;
  const transaction = db.transaction.bind(db);
  let writes = 0;
  db.transaction = ((...args: Parameters<typeof db.transaction>) => {
    if (args[1] === "readwrite") writes++;
    return transaction(...args);
  }) as typeof db.transaction;
  await internals.enqueueReconcile("A", [
    ...Array.from({ length: 1000 }, (_, i) => ({ id: String(i) })),
    { id: "blocked" },
  ], ["old"], 20);
  expect(writes).toBe(1);
  const saved = await persistence.loadSnapshot();
  expect(saved.entities.A).toHaveLength(1000);
  expect(saved.entities.A.some((row) => row.id === "blocked" || row.id === "old")).toBe(false);
  expect(saved.cursor).toEqual({ last_seq: 20 });
});

test("reconcile failure freezes later cursor writes and reset fences queued work", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  await persistence.saveCursor({ last_seq: 0 });
  const saveBatch = persistence.saveBatch.bind(persistence);
  persistence.saveBatch = async () => false;
  await internals.enqueueReconcile("A", [{ id: "1" }], [], 1);
  expect(engine.store.get("A", "1")).not.toBeNull();
  expect(internals.persistDegraded).toBe(true);
  persistence.saveBatch = saveBatch;
  await internals.enqueueApply([{ ...change("A", "2"), seq: 2 }], { last_seq: 2 });
  expect(engine.getCursor().last_seq).toBe(2);
  expect((await persistence.loadSnapshot()).cursor?.last_seq).toBe(0);
  let release!: () => void;
  internals.applyQueue = new Promise<void>((resolve) => { release = resolve; });
  const pending = internals.enqueueReconcile("A", [{ id: "stale" }], [], 1);
  internals.replicaEpoch++;
  engine.store.clearAll();
  release();
  await pending;
  expect(engine.store.list("A")).toEqual([]);
});

test("a late reconcile failure cannot change persistence state after reset", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  let release!: (durable: boolean) => void;
  persistence.saveBatch = () => new Promise<boolean>((resolve) => { release = resolve; });
  const pending = internals.enqueueReconcile("A", [{ id: "old" }], [], 1);
  await Promise.resolve();
  internals.replicaEpoch++;
  engine.store.clearAll();
  await persistence.clear();
  release(false);
  await pending;
  expect(internals.persistDegraded).toBe(false);
  expect(engine.store.list("A")).toEqual([]);
});

test("reconcile single-row fallback stops if reset happens during persistence", async () => {
  const persistence = await disk();
  Object.defineProperty(persistence, "saveBatch", { value: undefined });
  const { engine, internals } = reconcileEngine(persistence);
  let writes = 0;
  engine.store._persistFn = async () => {
    writes++;
    internals.replicaEpoch++;
    engine.store.clearAll();
    return true;
  };
  await internals.enqueueReconcile("A", [{ id: "1" }, { id: "2" }], [], 1);
  expect(writes).toBe(1);
  expect(engine.store.list("A")).toEqual([]);
});

test("1000 queued writes and acknowledgements persist linear entry counts", async () => {
  const persistence = new IndexedDBMutationPersistence(await disk());
  const queue = new MutationQueue(persistence);
  const saveChanges = persistence.saveChanges.bind(persistence);
  let puts = 0;
  let deletes = 0;
  let batches = 0;
  persistence.saveChanges = async (upserts, removed) => {
    puts += upserts.length;
    deletes += removed.length;
    batches++;
    await saveChanges(upserts, removed);
  };
  const ids = Array.from({ length: 1000 }, (_, i) => queue.add({
    entity: "A", row_id: String(i), kind: "insert", data: { id: String(i) },
  }, undefined, "user-1"));
  expect((await persistence.loadAll())).toHaveLength(1000);
  expect(puts).toBe(1000);
  queue.markAppliedMany(ids);
  expect((await persistence.loadAll()).every((m) => m.status === "applied")).toBe(true);
  expect(puts).toBe(2000);
  expect(batches).toBe(1001);
  queue.clear();
  expect(await persistence.loadAll()).toEqual([]);
  expect(deletes).toBe(1000);
});

test("incremental writes preserve failures and owners across hydration and reset", async () => {
  const persistence = new IndexedDBMutationPersistence(await disk());
  const queue = new MutationQueue(persistence);
  const failed = queue.add({ entity: "A", row_id: "1", kind: "delete" }, { id: "1" }, undefined, true);
  const pending = queue.add({ entity: "A", row_id: "2", kind: "delete" }, null, undefined, true);
  queue.stampPendingOwner("owner-1");
  queue.markFailed(failed, "denied", "POLICY_DENIED");
  const reloaded = new MutationQueue(persistence);
  await reloaded.hydrate();
  expect(reloaded.get(failed)).toMatchObject({ status: "failed", owner: "owner-1", errorCode: "POLICY_DENIED", prevRow: { id: "1" } });
  expect(reloaded.get(pending)).toMatchObject({ status: "pending", owner: "owner-1" });
  expect(reloaded.get(pending)?.ownerPending).toBeUndefined();
  reloaded.remove(failed);
  expect((await persistence.loadAll()).map((m) => m.id)).toEqual([pending]);
  reloaded.clearAll();
  expect(await persistence.loadAll()).toEqual([]);
});

test("custom mutation backends keep full snapshots and batch acknowledgements", async () => {
  const snapshots: unknown[][] = [];
  const queue = new MutationQueue({
    saveAll: async (rows) => { snapshots.push(rows); }, loadAll: async () => [],
  });
  const a = queue.add({ entity: "A", row_id: "1", kind: "delete" });
  const b = queue.add({ entity: "A", row_id: "2", kind: "delete" });
  queue.markAppliedMany([a, b]);
  expect(snapshots).toHaveLength(3);
  expect(snapshots[1]).toEqual(expect.arrayContaining([expect.objectContaining({ id: a, status: "pending" })]));
  expect(snapshots[2]).toEqual(expect.arrayContaining([expect.objectContaining({ id: a, status: "applied" })]));
});

test("queue ID operations do not visit unrelated mutations", () => {
  const queue = new MutationQueue();
  let idReads = 0;
  for (let i = 0; i < 1000; i++) {
    const id = `op-${i}`;
    queue.add({ op_id: id, entity: "A", row_id: String(i), kind: "delete" });
    Object.defineProperty(queue.get(id)!, "id", { get() { idReads++; return id; } });
  }
  const retained = queue.get("op-999");
  queue.add({ op_id: "op-999", entity: "A", row_id: "duplicate", kind: "delete" });
  queue.markFailed("op-999", "denied");
  expect(queue.get("op-999")).toBe(retained);
  queue.markAppliedMany(["op-999", "missing", "op-999"]);
  expect(retained?.status).toBe("applied");
  queue.remove("op-998");
  queue.discard("op-997");
  expect(idReads).toBeLessThan(10);
  expect(queue.pending()).toHaveLength(997);
});

test("hydration preserves live entry identity and deduplicates stored IDs in order", async () => {
  let release!: (rows: import("./mutation-queue").PendingMutation[]) => void;
  const snapshots: import("./mutation-queue").PendingMutation[][] = [];
  const queue = new MutationQueue({
    loadAll: () => new Promise((resolve) => { release = resolve; }),
    saveAll: async (rows) => { snapshots.push(rows); },
  });
  const hydration = queue.hydrate();
  const makeChange = (id: string) => ({ op_id: id, entity: "A", row_id: id, kind: "delete" as const });
  queue.add(makeChange("live"));
  const live = queue.get("live");
  release([
    { id: "live", change: makeChange("live"), status: "failed" },
    { id: "saved", change: makeChange("saved"), status: "pending" },
    { id: "saved", change: makeChange("saved"), status: "failed" },
  ]);
  await hydration;
  expect(queue.get("live")).toBe(live);
  expect(queue.pending().map((m) => m.id)).toEqual(["live", "saved"]);
  queue.markAppliedMany(["saved", "live"]);
  expect(snapshots.at(-1)?.map((m) => m.id)).toEqual(["live", "saved"]);
  queue.clear();
  expect(snapshots.at(-1)).toEqual([]);
});


test("dropping 1000 entity rows uses one transaction and preserves cursor and tombstones", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  const rows = Array.from({ length: 1000 }, (_, i) => ({ id: String(i) }));
  await internals.enqueueReconcile("A", rows, [], 20);
  await persistence.saveCursor({ last_seq: 20 });
  const db = persistence.connection!;
  const transaction = db.transaction.bind(db);
  let writes = 0;
  db.transaction = ((...args: Parameters<typeof db.transaction>) => {
    if (args[1] === "readwrite") writes++;
    return transaction(...args);
  }) as typeof db.transaction;
  await internals.dropEntity("A", 20);
  expect(writes).toBe(1);
  expect(engine.store.list("A")).toEqual([]);
  const saved = await persistence.loadSnapshot();
  expect(saved.entities.A ?? []).toEqual([]);
  expect(saved.cursor).toEqual({ last_seq: 20 });
  engine.store.applyChange({ ...change("A", "0"), seq: 19 });
  expect(engine.store.get("A", "0")).toBeNull();
  engine.store.applyChange({ ...change("A", "0"), seq: 21 });
  expect(engine.store.get("A", "0")).not.toBeNull();
});

test("queued entity removal cannot delete rows after replica reset", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  await internals.enqueueReconcile("A", [{ id: "same" }], [], 1);
  let release!: () => void;
  internals.applyQueue = new Promise<void>((resolve) => { release = resolve; });
  const pending = internals.dropEntity("A", 10);
  internals.replicaEpoch++;
  engine.store.clearAll();
  engine.store.applyChange(change("A", "same"));
  release();
  await pending;
  expect(engine.store.get("A", "same")).not.toBeNull();
});


test("entity removal failure freezes persistence and fallback stops after reset", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  engine.store.applyInMemory([change("A", "1"), change("A", "2")]);
  persistence.saveBatch = async () => false;
  await internals.dropEntity("A", 10);
  expect(internals.persistDegraded).toBe(true);
  expect(engine.store.list("A")).toEqual([]);
  Object.defineProperty(persistence, "saveBatch", { value: undefined });
  engine.store.applyInMemory([
    { ...change("A", "1"), seq: 11 },
    { ...change("A", "2"), seq: 11 },
  ]);
  let deletes = 0;
  engine.store._persistFn = async () => {
    deletes++;
    internals.replicaEpoch++;
    engine.store.clearAll();
    return true;
  };
  await internals.dropEntity("A", 20);
  expect(deletes).toBe(1);
});


test("a thrown removal batch freezes the disk cursor and still broadcasts removal", async () => {
  const persistence = await disk();
  const { engine, internals } = reconcileEngine(persistence);
  engine.store.applyChange(change("A", "1"));
  persistence.saveBatch = async () => { throw new Error("disk failed"); };
  const messages: unknown[] = [];
  (engine as unknown as { broadcastToTabs(message: unknown): void }).broadcastToTabs = (message) => messages.push(message);
  await internals.dropEntity("A", 10);
  expect(internals.persistDegraded).toBe(true);
  expect(engine.store.list("A")).toEqual([]);
  expect(messages).toEqual([{ type: "reconciled", entity: "A", upserts: [], removalIds: ["1"], tombstoneSeq: 10 }]);
});
