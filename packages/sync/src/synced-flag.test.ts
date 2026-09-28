// `isSynced()` tells a warm-but-incomplete replica apart from one the
// server has confirmed. `isInitialSyncSettled()` flips true as soon as the
// IndexedDB cache holds rows (and after a 12 s fallback), so apps rendered
// stale partial cache data as if it were complete (saas template).

import { afterEach, describe, expect, test } from "bun:test";

import { SyncEngine, type ReplicaPersistence, type Row, type SyncCursor } from "./index";
import { createTestEnv, type TestEnv } from "./test-harness";

class WarmCache implements ReplicaPersistence {
  constructor(private rows: Record<string, Row[]>, private cursor: SyncCursor) {}
  async open() {}
  async loadSnapshot() {
    return { entities: this.rows, cursor: this.cursor, hadCache: true };
  }
  async loadIdentity() {
    return undefined;
  }
  async saveIdentity() {
    return true;
  }
  async saveCursor() {
    return true;
  }
  async saveRow() {
    return true;
  }
  async deleteRow() {
    return true;
  }
  async clear() {
    return true;
  }
}

describe("isSynced", () => {
  let env: TestEnv | null = null;
  const realFetch = globalThis.fetch;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
    globalThis.fetch = realFetch;
  });

  test("a warm cache settles loading but is not synced until the pull lands", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    globalThis.fetch = (async (url: string) => {
      if (String(url).includes("/api/sync/pull")) {
        await gate;
        return new Response(
          JSON.stringify({ changes: [], cursor: { last_seq: 5 }, has_more: false }),
          { status: 200 },
        );
      }
      if (String(url).includes("/api/auth/me")) {
        return new Response(JSON.stringify({ user_id: null }), { status: 200 });
      }
      return new Response("{}", { status: 404 });
    }) as typeof fetch;
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      multiTab: false,
      transport: "poll",
      pollInterval: 60_000,
      persistence: new WarmCache({ Todo: [{ id: "t1", title: "cached" }] }, { last_seq: 3 }),
    });
    const started = engine.start();
    for (let i = 0; i < 20 && engine.store.list("Todo").length === 0; i++) {
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(engine.store.list("Todo")).toHaveLength(1);
    expect(engine.isInitialSyncSettled()).toBe(true);
    expect(engine.isSynced()).toBe(false);

    release();
    await started;
    expect(engine.isSynced()).toBe(true);
    engine.stop();
  });

  test("a failed pull leaves it false; a reset clears it until the re-pull", async () => {
    let failPulls = true;
    env = createTestEnv({
      transport: "poll",
      beforePull: () => {
        if (failPulls) throw new Error("server down");
      },
    });
    env.signIn({ userId: "u1" });
    await env.start();
    expect(env.engine.isSynced()).toBe(false);

    failPulls = false;
    await env.engine.pull();
    expect(env.engine.isSynced()).toBe(true);

    await env.engine.resetReplica();
    expect(env.engine.isSynced()).toBe(false);
    await env.engine.pull();
    expect(env.engine.isSynced()).toBe(true);
  });

  test("a follower tab takes it from the leader's synced message", async () => {
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const internals = engine as unknown as {
      isMultiTabLeader: boolean;
      multiTabHooks: () => { onSyncedReceived: () => void };
    };
    internals.isMultiTabLeader = false;
    expect(engine.isSynced()).toBe(false);
    internals.multiTabHooks().onSyncedReceived();
    await new Promise((r) => setTimeout(r, 0));
    expect(engine.isSynced()).toBe(true);
  });
});
