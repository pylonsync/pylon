// Pluggable replica persistence (SyncEngineConfig.persistence).
//
// Non-browser hosts (React Native, Tauri) have no IndexedDB — before this
// config existed their replica was memory-only and a cold offline launch
// rendered an empty store. These tests drive the engine with a MEMORY
// adapter and prove:
//
//   1. A cached replica hydrates the store on start() with the network
//      DOWN — the offline-gym cold start.
//   2. Applied changes flow through the adapter (saveRow), so the next
//      cold start has them.

import { afterEach, beforeEach, expect, test } from "bun:test";
import {
  SyncEngine,
  type ReplicaPersistence,
  type Row,
  type SyncCursor,
} from "./index";

class MemoryPersistence implements ReplicaPersistence {
  entities = new Map<string, Map<string, Row>>();
  cursor: SyncCursor | null = null;
  identity: string | null | undefined = undefined;
  savedRows: Array<[string, string]> = [];

  async open(): Promise<void> {}
  async loadSnapshot() {
    const entities: Record<string, Row[]> = {};
    for (const [e, rows] of this.entities) entities[e] = [...rows.values()];
    return {
      entities,
      cursor: this.cursor,
      hadCache: this.entities.size > 0 || this.cursor != null,
    };
  }
  async loadIdentity() {
    return this.identity;
  }
  async saveIdentity(userId: string | null) {
    this.identity = userId;
    return true;
  }
  async saveCursor(cursor: SyncCursor) {
    this.cursor = cursor;
    return true;
  }
  async saveRow(entity: string, id: string, data: Row) {
    let m = this.entities.get(entity);
    if (!m) {
      m = new Map();
      this.entities.set(entity, m);
    }
    m.set(id, data);
    this.savedRows.push([entity, id]);
    return true;
  }
  async deleteRow(entity: string, id: string) {
    this.entities.get(entity)?.delete(id);
    return true;
  }
  async clear() {
    this.entities.clear();
    this.cursor = null;
    return true;
  }
}

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function offlineFetch(): typeof fetch {
  return (() =>
    Promise.reject(new TypeError("network down"))) as unknown as typeof fetch;
}

function makeEngine(persistence: ReplicaPersistence): SyncEngine {
  return new SyncEngine({
    baseUrl: "http://sync-test.invalid",
    transport: "poll",
    persistence,
    appName: "custom-persistence-test",
  });
}

let engine: SyncEngine | null = null;
beforeEach(() => {
  engine = null;
});
afterEach(async () => {
  await engine?.stop();
});

test("cold OFFLINE start hydrates the store from a custom adapter", async () => {
  const mem = new MemoryPersistence();
  mem.identity = "user-1";
  mem.cursor = { last_seq: 42 };
  mem.entities.set(
    "Workout",
    new Map([
      ["w1", { id: "w1", title: "Cached Leg Day", ownerId: "user-1" }],
      ["w2", { id: "w2", title: "Cached Push Day", ownerId: "user-1" }],
    ]),
  );

  globalThis.fetch = offlineFetch();
  engine = makeEngine(mem);
  await engine.start();

  const rows = engine.store.list("Workout");
  expect(rows.length).toBe(2);
  expect(rows.map((r) => r.title).sort()).toEqual([
    "Cached Leg Day",
    "Cached Push Day",
  ]);
});

test("applied changes persist through the adapter", async () => {
  const mem = new MemoryPersistence();
  globalThis.fetch = offlineFetch();
  engine = makeEngine(mem);
  await engine.start();

  await engine.store.applyChangesAsync([
    {
      kind: "insert",
      entity: "Workout",
      row_id: "w9",
      seq: 1,
      data: { id: "w9", title: "New Session" },
      timestamp: "",
    },
  ]);
  // Adapter writes settle on the applyChanges await (the engine passes
  // each change through persistChange → saveRow).
  expect(mem.savedRows.some(([e, id]) => e === "Workout" && id === "w9")).toBe(true);

  // ...and the NEXT engine (fresh memory) sees it on a cold offline start.
  await engine.stop();
  engine = makeEngine(mem);
  await engine.start();
  expect(engine.store.list("Workout").some((r) => r.id === "w9")).toBe(true);
});
