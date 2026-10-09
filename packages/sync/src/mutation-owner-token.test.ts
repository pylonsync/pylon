// Owner check edge cases found in the third review: a push between a
// token swap and the session refresh, writes made before any identity is
// known, a session that expired across a reload, and guest -> user.

import "fake-indexeddb/auto";
import { afterEach, describe, expect, test } from "bun:test";

import { IndexedDBPersistence, SyncEngine } from "./index";
import { IndexedDBMutationPersistence } from "./persistence";
import { createTestEnv, type TestEnv } from "./test-harness";

describe("mutation owner and the bearer token", () => {
  let env: TestEnv | null = null;
  const realFetch = globalThis.fetch;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
    globalThis.fetch = realFetch;
  });

  test("a push after a token swap and before the refresh does not send A's write with B's token", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "uA" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    const id = await env.engine.insert("Note", { title: "A's draft" });
    const pushAuth: string[] = [];
    const inner = globalThis.fetch;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).includes("/api/sync/push")) {
        const h = (init?.headers ?? {}) as Record<string, string>;
        pushAuth.push(h.Authorization ?? "");
      }
      return inner(input, init);
    }) as typeof fetch;
    // B's token is stored; nothing has refreshed the session yet.
    const tokenB = env.signIn({ userId: "uB" });
    await env.engine.push();
    expect(pushAuth).not.toContain(`Bearer ${tokenB}`);
    // Once the session says B, A's write is discarded, never sent.
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.server.receivedPushKeys.filter((k) => k === `Note/${id}`)).toHaveLength(1);
    expect(env.engine.pendingCount()).toBe(0);
  });

  test("a guest's queued write is pushed after the guest signs in", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "guest_1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    const id = await env.engine.insert("Note", { title: "guest note" });
    env.signIn({ userId: "u1" });
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.engine.pendingCount()).toBe(0);
    expect(env.server.receivedPushKeys.filter((k) => k === `Note/${id}`)).toHaveLength(2);
  });

  test("a write made before any identity is known waits for the first session and takes its owner", async () => {
    let meCalls = 0;
    let pushes = 0;
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/auth/me")) {
        meCalls += 1;
        if (meCalls <= 2) throw new TypeError("offline");
        return new Response(JSON.stringify({ user_id: "u1" }), { status: 200 });
      }
      if (url.includes("/api/sync/push")) {
        pushes += 1;
        return new Response(JSON.stringify({ applied: 1, errors: [], results: [], cursor: { last_seq: 0 } }), { status: 200 });
      }
      return new Response(JSON.stringify({ changes: [], cursor: { last_seq: 0 }, has_more: false }), { status: 200 });
    }) as typeof fetch;
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      persist: false,
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
    });
    await engine.start();
    const insert = engine.insert("Note", { title: "offline" });
    await new Promise((r) => setTimeout(r, 0));
    expect(engine.mutations.pending()[0]!.ownerPending).toBe(true);
    await insert;
    expect(pushes).toBe(0);
    for (let i = 0; i < 40 && pushes === 0; i++) await new Promise((r) => setTimeout(r, 50));
    expect(pushes).toBeGreaterThan(0);
    engine.stop();
  }, 10_000);

  test("a session that expired across a reload keeps the user's queued writes", async () => {
    let pushCalls = 0;
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/sync/push")) {
        pushCalls += 1;
        throw new Error("must not push");
      }
      const body = url.includes("/api/auth/me")
        ? { user_id: null }
        : { changes: [], cursor: { last_seq: 0 }, has_more: false };
      return new Response(JSON.stringify(body), { status: 200 });
    }) as typeof fetch;
    const name = `expired-${Math.random().toString(36).slice(2)}`;
    const disk = new IndexedDBPersistence(name);
    await disk.open();
    await disk.saveIdentity("u1");
    await disk.saveCursor({ last_seq: 1 });
    await new IndexedDBMutationPersistence(disk).saveAll([
      {
        id: "op-1",
        change: { entity: "Note", row_id: "n1", kind: "insert", data: { id: "n1" }, op_id: "op-1" },
        status: "pending",
        owner: "u1",
      },
    ]);
    disk.connection?.close();
    const reopened = new IndexedDBPersistence(name);
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      persistence: reopened,
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
    });
    await engine.start();
    await engine.push();
    expect(pushCalls).toBe(0);
    expect(engine.pendingCount()).toBe(1);
    engine.stop();
    reopened.connection?.close();
  });
});
