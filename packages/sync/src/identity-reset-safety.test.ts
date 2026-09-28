// Identity changes must not wipe a follower twice, let the old socket's
// frames into the new replica, or throw away a signed-out user's offline
// writes. Found in review of the sign-in re-sync fix.

import "fake-indexeddb/auto";
import { afterEach, describe, expect, test } from "bun:test";

import { IndexedDBPersistence, SyncEngine, type ChangeEvent } from "./index";
import { createTestEnv, type TestEnv } from "./test-harness";

const session = (userId: string | null) => ({
  userId,
  tenantId: null,
  isAdmin: false,
  roles: [],
  avatarUrl: null,
});

describe("identity reset safety", () => {
  let env: TestEnv | null = null;
  const realFetch = globalThis.fetch;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
    globalThis.fetch = realFetch;
  });

  test("a follower keeps the leader's re-pulled rows when the session broadcast arrives", async () => {
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const internals = engine as unknown as {
      isMultiTabLeader: boolean;
      multiTabHooks: () => { onSessionReceived: (s: unknown) => void };
      sessionChain: Promise<void>;
      enqueueApply: (c: unknown[], t?: unknown, o?: unknown) => Promise<void>;
    };
    internals.isMultiTabLeader = false;
    const hooks = internals.multiTabHooks();
    hooks.onSessionReceived(session(null));
    await internals.sessionChain;
    // The leader reset, re-pulled, and broadcast the rows...
    await internals.enqueueApply(
      [{ seq: 5, entity: "Deal", row_id: "d1", kind: "insert", data: { id: "d1" }, timestamp: "" }],
      { last_seq: 5 },
      { fromBroadcast: true },
    );
    // ...then its session.
    hooks.onSessionReceived(session("u1"));
    await internals.sessionChain;
    await new Promise((r) => setTimeout(r, 20));
    expect(engine.store.list("Deal")).toHaveLength(1);
    expect(engine.getCursor().last_seq).toBe(5);
    expect(engine.resolvedSession().userId).toBe("u1");
  });

  test("a frame from the old socket during a reset does not land in the new replica", async () => {
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      const body = url.includes("/api/sync/pull")
        ? { changes: [], cursor: { last_seq: 0 }, has_more: false }
        : url.includes("/api/auth/me")
          ? { user_id: "uA" }
          : {};
      return new Response(JSON.stringify(body), { status: 200 });
    }) as typeof fetch;
    const persistence = new IndexedDBPersistence(`leak-${Math.random().toString(36).slice(2)}`);
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      persistence,
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
      reconcileOnVisibility: false,
    });
    await engine.start();
    const host = (engine as unknown as {
      transportHost(): { onChangeEvent(ev: ChangeEvent): void };
    }).transportHost();
    // A batch that starts before the reset and is still writing when it runs.
    host.onChangeEvent({ seq: 800, entity: "Secret", row_id: "s0", kind: "insert", data: { id: "s0" }, timestamp: "" });
    // Hold the reset inside its disk clear so the next frame arrives while
    // it runs.
    let releaseClear!: () => void;
    const clearGate = new Promise<void>((r) => (releaseClear = r));
    const realClear = persistence.clear.bind(persistence);
    let clearStarted = false;
    persistence.clear = async () => {
      clearStarted = true;
      await clearGate;
      return realClear();
    };
    const reset = engine.resetReplica({ wipeMutations: true });
    while (!clearStarted) await new Promise((r) => setTimeout(r, 1));
    host.onChangeEvent({ seq: 900, entity: "Secret", row_id: "s1", kind: "insert", data: { id: "s1" }, timestamp: "" });
    releaseClear();
    await reset;
    await new Promise((r) => setTimeout(r, 50));
    expect(engine.store.list("Secret")).toHaveLength(0);
    expect(engine.getCursor().last_seq).toBe(0);
    const disk = await persistence.loadSnapshot();
    expect(disk.entities.Secret ?? []).toHaveLength(0);
    expect(disk.cursor?.last_seq ?? 0).toBe(0);
    engine.stop();
  });

  test("an expired session holds offline writes; the same user signing back in pushes them", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    const id = await env.engine.insert("Note", { title: "offline draft" });
    expect(env.engine.pendingCount()).toBe(1);

    // The token expires: /api/auth/me now answers anonymous.
    if (env.token) env.server.revoke(env.token);
    await env.engine.notifySessionChanged();
    await env.flush();
    expect(env.engine.resolvedSession().userId).toBeNull();
    expect(env.engine.pendingCount()).toBe(1);
    await env.engine.push();
    expect(env.server.receivedPushKeys.filter((k) => k === `Note/${id}`)).toHaveLength(1);

    env.signIn({ userId: "u1" });
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.engine.pendingCount()).toBe(0);
    expect(env.server.receivedPushKeys.filter((k) => k === `Note/${id}`)).toHaveLength(2);
  });

  test("a different user signing in after an expired session discards the held writes", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Note", { title: "u1 draft" });
    if (env.token) env.server.revoke(env.token);
    await env.engine.notifySessionChanged();
    await env.flush();

    env.signIn({ userId: "u2" });
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.engine.pendingCount()).toBe(0);
    expect(env.server.receivedPushKeys.filter((k) => k.startsWith("Note/"))).toHaveLength(1);
  });

  test("a follower's write resolves as queued when the leader never answers", async () => {
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const internals = engine as unknown as {
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
    };
    internals.isMultiTabLeader = false;
    internals.broadcastToTabs = () => {};
    const started = Date.now();
    const id = await engine.insert("Note", { title: "x" });
    expect(typeof id).toBe("string");
    expect(Date.now() - started).toBeGreaterThanOrEqual(4_900);
    expect(engine.store.get("Note", id)).not.toBeNull();
  }, 10_000);

  test("an op the server returns no result for resolves as queued", async () => {
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/sync/push")) {
        return new Response(JSON.stringify({ applied: 0, errors: [], results: [], cursor: { last_seq: 0 } }), { status: 200 });
      }
      return new Response("{}", { status: 200 });
    }) as typeof fetch;
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const id = await engine.insert("Note", { title: "x" });
    expect(engine.store.get("Note", id)).not.toBeNull();
    expect(engine.pendingCount()).toBe(1);
  });
});
