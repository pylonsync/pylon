// The outbox: list pending and failed writes, retry a failed write (with
// corrected fields), dismiss one, and notify subscribers on every change.
//
// Before: a write the server rejected was rolled back and kept in the queue
// as `failed`, but nothing listed it, so the user never learned it was lost.

import "fake-indexeddb/auto";
import { afterEach, describe, expect, test } from "bun:test";

import {
  IndexedDBPersistence,
  MutationRejectedError,
  OUTBOX_CHANNEL,
  SyncEngine,
} from "./index";
import {
  MutationQueue,
  type MutationQueuePersistence,
  type PendingMutation,
} from "./mutation-queue";
import { createTestEnv, type TestEnv } from "./test-harness";

/** In-memory stand-in for the IndexedDB queue store. */
class MemoryQueueStore implements MutationQueuePersistence {
  rows = new Map<string, PendingMutation>();
  async saveAll(mutations: PendingMutation[]) {
    this.rows = new Map(mutations.map((m) => [m.id, structuredClone(m)]));
  }
  async loadAll() {
    return Array.from(this.rows.values(), (m) => structuredClone(m));
  }
  async saveChanges(upserts: PendingMutation[], removedIds: string[]) {
    for (const m of upserts) this.rows.set(m.id, structuredClone(m));
    for (const id of removedIds) this.rows.delete(id);
  }
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("MutationQueue", () => {
  test("a failed write survives clear() and a reload", async () => {
    const disk = new MemoryQueueStore();
    const q = new MutationQueue(disk);
    const applied = q.add({ entity: "Knock", row_id: "k1", kind: "insert", data: { id: "k1" } }, undefined, "u1");
    const failed = q.add({ entity: "Knock", row_id: "k2", kind: "insert", data: { id: "k2" } }, undefined, "u1");
    q.markApplied(applied);
    q.markFailed(failed, "door number is required", "VALIDATION_FAILED");
    q.clear();
    await tick();
    expect(q.failed().map((m) => m.id)).toEqual([failed]);

    const reloaded = new MutationQueue(disk);
    await reloaded.hydrate();
    const [m] = reloaded.failed();
    expect(m).toMatchObject({
      id: failed,
      status: "failed",
      error: "door number is required",
      errorCode: "VALIDATION_FAILED",
      owner: "u1",
    });
    expect(typeof m!.createdAt).toBe("number");
    expect(reloaded.get(applied)).toBeUndefined();
  });

  test("retry re-queues as pending under a new op id, with the owner and merged fields", async () => {
    const disk = new MemoryQueueStore();
    const q = new MutationQueue(disk);
    const id = q.add(
      { entity: "Knock", row_id: "k1", kind: "insert", data: { id: "k1", door: "", outcome: "no answer" } },
      undefined,
      "u1",
    );
    const createdAt = q.get(id)!.createdAt;
    q.markFailed(id, "door number is required", "VALIDATION_FAILED");
    const newId = q.retry(id, { door: "14B", id: "attempt-to-change-id" }, undefined);
    await tick();

    expect(newId).not.toBe(id);
    expect(q.get(id)).toBeUndefined();
    const m = q.get(newId)!;
    expect(m.status).toBe("pending");
    expect(m.owner).toBe("u1");
    expect(m.createdAt).toBe(createdAt);
    expect(m.change.op_id).toBe(newId);
    expect(m.change.data).toEqual({ id: "k1", door: "14B", outcome: "no answer" });
    // One persistence write removed the old entry and added the new one.
    expect(Array.from(disk.rows.keys())).toEqual([newId]);
    expect(() => q.retry(newId, undefined, undefined)).toThrow();
  });

  test("dismiss removes a failed write from memory and persistence; a pending one stays", async () => {
    const disk = new MemoryQueueStore();
    const q = new MutationQueue(disk);
    const failed = q.add({ entity: "Knock", row_id: "k1", kind: "insert", data: { id: "k1" } });
    const pending = q.add({ entity: "Knock", row_id: "k2", kind: "insert", data: { id: "k2" } });
    q.markFailed(failed, "no");
    expect(q.dismiss(pending)).toBe(false);
    expect(q.dismiss(failed)).toBe(true);
    await tick();
    expect(q.get(failed)).toBeUndefined();
    expect(Array.from(disk.rows.keys())).toEqual([pending]);
  });

  test("writes from an older client (no createdAt) sort first", () => {
    const q = new MutationQueue();
    const newer = q.add({ entity: "K", row_id: "a", kind: "insert", data: { id: "a" } });
    const older = q.add({ entity: "K", row_id: "b", kind: "insert", data: { id: "b" } });
    delete q.get(older)!.createdAt;
    q.markFailed(newer, "x");
    q.markFailed(older, "y");
    expect(q.failed().map((m) => m.id)).toEqual([older, newer]);
  });
});

describe("SyncEngine outbox", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  test("lists pending and failed writes with their fields and errors", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.rejectWritesWhere("Knock", (d) => d.door === "", "VALIDATION_FAILED", "door number is required");
    await expect(env.engine.insert("Knock", { door: "", outcome: "no answer" })).rejects.toThrow();
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Knock", { door: "12", outcome: "sold" });

    const { pending, failed } = env.engine.outbox();
    expect(pending).toHaveLength(1);
    expect(pending[0]).toMatchObject({ entity: "Knock", kind: "insert", data: { door: "12", outcome: "sold" } });
    expect(pending[0]!.error).toBeUndefined();
    expect(failed).toHaveLength(1);
    expect(failed[0]).toMatchObject({
      entity: "Knock",
      kind: "insert",
      data: { door: "", outcome: "no answer" },
      error: "door number is required",
      errorCode: "VALIDATION_FAILED",
    });
    // Same object until the queue changes.
    expect(env.engine.outbox()).toBe(env.engine.outbox());
  });

  test("subscribers hear add, applied, failed, retry and dismiss", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    let calls = 0;
    env.engine.store.subscribe(() => {
      calls += 1;
    }, OUTBOX_CHANNEL);
    const heard = async (fn: () => Promise<unknown> | unknown) => {
      const before = calls;
      try {
        await fn();
      } catch {
        // A rejected write still changes the outbox.
      }
      await env!.flush(5);
      return calls > before;
    };

    // add + applied
    expect(await heard(() => env!.engine.insert("Knock", { door: "1" }))).toBe(true);
    // add + failed
    env.server.rejectWritesWhere("Knock", (d) => d.door === "", "VALIDATION_FAILED", "required");
    expect(await heard(() => env!.engine.insert("Knock", { door: "" }))).toBe(true);
    const failedId = env.engine.outbox().failed[0]!.id;
    // retry (fails again)
    expect(await heard(() => env!.engine.retryWrite(failedId))).toBe(true);
    // dismiss
    const again = env.engine.outbox().failed[0]!.id;
    expect(await heard(() => env!.engine.dismissWrite(again))).toBe(true);
    expect(env.engine.outbox().failed).toHaveLength(0);

    // Entity listeners are not woken by outbox-only changes.
    let entityCalls = 0;
    env.engine.store.subscribe(() => {
      entityCalls += 1;
    }, "Other");
    env.engine.mutations.markFailed("nope", "x");
    env.engine.store.notify(OUTBOX_CHANNEL);
    expect(entityCalls).toBe(0);
  });

  test("retry with corrected data is applied; the rejected op is never applied", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.rejectWritesWhere("Knock", (d) => d.door === "", "VALIDATION_FAILED", "door number is required");

    let rowId = "";
    try {
      await env.engine.insert("Knock", { door: "", outcome: "sold" });
    } catch (err) {
      rowId = (err as MutationRejectedError).rowId;
    }
    // The rejection rolled the row back.
    expect(env.engine.store.get("Knock", rowId)).toBeNull();
    const failed = env.engine.outbox().failed[0]!;

    const newOp = await env.engine.retryWrite(failed.id, { door: "14B" });
    expect(newOp).not.toBe(failed.id);
    expect(env.engine.store.get("Knock", rowId)).toMatchObject({ door: "14B", outcome: "sold" });
    expect(env.engine.outbox()).toEqual({ pending: [], failed: [] });
    const onServer = await env.server.listEntityRows("Knock", env.token);
    expect(onServer).toEqual([{ id: rowId, door: "14B", outcome: "sold" }]);
  });

  test("a retry the server rejects again is failed again with the new error", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.seed("Knock", [{ id: "k1", door: "3", outcome: "no answer" }]);
    await env.engine.pull();
    env.server.rejectWritesWhere("Knock", (d) => d.outcome === "bogus", "VALIDATION_FAILED", "unknown outcome");
    env.server.rejectWritesWhere("Knock", (d) => d.outcome === "worse", "POLICY_DENIED", "not allowed");
    await expect(env.engine.update("Knock", "k1", { outcome: "bogus" })).rejects.toThrow();
    expect(env.engine.store.get("Knock", "k1")).toMatchObject({ outcome: "no answer" });

    const id = env.engine.outbox().failed[0]!.id;
    await expect(env.engine.retryWrite(id, { outcome: "worse" })).rejects.toMatchObject({
      code: "POLICY_DENIED",
    });
    const [failed] = env.engine.outbox().failed;
    expect(failed).toMatchObject({ errorCode: "POLICY_DENIED", error: "not allowed", data: { outcome: "worse" } });
    expect(failed!.id).not.toBe(id);
    // Rolled back to the server's value again.
    expect(env.engine.store.get("Knock", "k1")).toMatchObject({ outcome: "no answer" });
  });

  test("a failed write is never retried as another user", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.denyWrites("Knock", "POLICY_DENIED", "no");
    await expect(env.engine.insert("Knock", { door: "1" })).rejects.toThrow();
    const id = env.engine.outbox().failed[0]!.id;

    env.signIn({ userId: "u2" });
    await env.engine.refreshResolvedSession();
    await env.flush(10);
    await expect(env.engine.retryWrite(id)).rejects.toMatchObject({ code: "OWNER_MISMATCH" });
    expect(env.server.receivedPushKeys.filter((k) => k.startsWith("Knock/"))).toHaveLength(1);
  });

  test("retryWrite and dismissWrite refuse a write that is not failed", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Knock", { door: "1" });
    const id = env.engine.outbox().pending[0]!.id;
    await expect(env.engine.retryWrite(id)).rejects.toThrow("no failed write");
    expect(env.engine.dismissWrite(id)).toBe(false);
  });
});

describe("pending writes after a reload", () => {
  const realFetch = globalThis.fetch;
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  test("rows written offline are on screen again before the network answers", async () => {
    // Offline: every request fails.
    globalThis.fetch = (async () => {
      throw new TypeError("Failed to fetch");
    }) as unknown as typeof fetch;
    const name = `reload-${Math.random().toString(36).slice(2)}`;
    const disk = new IndexedDBPersistence(name);
    await disk.open();
    await disk.saveIdentity("u1");
    await disk.saveCursor({ last_seq: 5 });
    await disk.saveRow("Knock", "k-old", { id: "k-old", door: "1", outcome: "no answer" });
    const { IndexedDBMutationPersistence } = await import("./persistence");
    const now = Date.now();
    await new IndexedDBMutationPersistence(disk).saveAll([
      {
        id: "op-insert",
        change: { entity: "Knock", row_id: "k-new", kind: "insert", data: { id: "k-new", door: "2", outcome: "sold" }, op_id: "op-insert" },
        status: "pending",
        owner: "u1",
        createdAt: now - 2000,
      },
      {
        id: "op-update",
        change: { entity: "Knock", row_id: "k-old", kind: "update", data: { outcome: "callback" }, op_id: "op-update" },
        status: "pending",
        owner: "u1",
        prevRow: { id: "k-old", door: "1", outcome: "no answer" },
        createdAt: now - 1000,
      },
      {
        id: "op-failed",
        change: { entity: "Knock", row_id: "k-bad", kind: "insert", data: { id: "k-bad" }, op_id: "op-failed" },
        status: "failed",
        error: "rejected",
        owner: "u1",
        createdAt: now - 500,
      },
    ]);

    disk.connection?.close();

    const reopened = new IndexedDBPersistence(name);
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      appName: name,
      persistence: reopened,
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
    });
    const started = engine.start().catch(() => {});
    for (let i = 0; i < 50 && !engine.isHydrated(); i++) await new Promise((r) => setTimeout(r, 10));
    await new Promise((r) => setTimeout(r, 20));
    expect(engine.store.get("Knock", "k-new")).toMatchObject({ door: "2", outcome: "sold" });
    expect(engine.store.get("Knock", "k-old")).toMatchObject({ outcome: "callback" });
    // A failed write was rolled back; it is not re-applied.
    expect(engine.store.get("Knock", "k-bad")).toBeNull();
    expect(engine.outbox().pending.map((m) => m.id)).toEqual(["op-insert", "op-update"]);
    expect(engine.outbox().failed.map((m) => m.id)).toEqual(["op-failed"]);
    engine.stop();
    await started;
    // An open connection blocks other suites' deleteDatabase.
    reopened.connection?.close();
  });
});
