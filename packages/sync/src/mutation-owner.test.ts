// Every queued write carries the user it was made as, and push only sends
// it as that user. Found in the second review of the identity-reset fix:
// a follower tab could forward user A's write for the leader to push as
// user B, a token change whose /api/auth/me failed held every write
// forever, and a reload while signed out pushed the held writes as the
// anonymous caller.

import "fake-indexeddb/auto";
import { afterEach, describe, expect, test } from "bun:test";

import { IndexedDBPersistence, SyncEngine, type ChangeEvent } from "./index";
import type { PendingMutation } from "./mutation-queue";
import { createTestEnv, type TestEnv } from "./test-harness";

const session = (userId: string | null) => ({
  userId,
  tenantId: null,
  isAdmin: false,
  roles: [],
  avatarUrl: null,
});

describe("mutation owner", () => {
  let env: TestEnv | null = null;
  const realFetch = globalThis.fetch;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
    globalThis.fetch = realFetch;
  });

  test("a write is tagged with the user who made it", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Note", { title: "x" });
    expect(env.engine.mutations.pending()[0]!.owner).toBe("u1");
  });

  test("the leader discards a forwarded write of another user; the follower drops it", async () => {
    // Leader, signed in as B.
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "uB" });
    await env.start();
    const leaderSent: unknown[] = [];
    (env.engine as unknown as { broadcastToTabs: (p: unknown) => void }).broadcastToTabs = (p) =>
      leaderSent.push(p);
    const forwarded: PendingMutation = {
      id: "op-a",
      change: { entity: "Note", row_id: "n-a", kind: "insert", data: { id: "n-a" }, op_id: "op-a" },
      status: "pending",
      owner: "uA",
    };
    (env.engine as unknown as {
      multiTabHooks: () => { onMutationsForwarded: (ops: PendingMutation[]) => void };
    })
      .multiTabHooks()
      .onMutationsForwarded([forwarded]);
    await env.flush(20);
    expect(env.server.receivedPushKeys).not.toContain("Note/n-a");
    expect(env.engine.pendingCount()).toBe(0);
    expect(leaderSent).toContainEqual({
      type: "mutations-failed",
      ops: [{ opId: "op-a", error: "discarded: another user is signed in", code: "DISCARDED" }],
    });

    // Follower that made the write: it drops the op without restoring rows.
    const follower = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const f = follower as unknown as {
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
      multiTabHooks: () => {
        onMutationsFailed: (ops: { opId: string; error: string; code?: string }[]) => void;
      };
    };
    f.isMultiTabLeader = false;
    f.broadcastToTabs = () => {};
    follower.store.applyChange({ seq: 1, entity: "Note", row_id: "n0", kind: "insert", data: { id: "n0", title: "A's row" }, timestamp: "" });
    const p = follower.update("Note", "n0", { title: "A edit" });
    await new Promise((r) => setTimeout(r, 0));
    // The leader's reset for B already wiped this tab's rows.
    follower.store.clearAll();
    const opId = follower.mutations.pending()[0]!.id;
    f.multiTabHooks().onMutationsFailed([{ opId, error: "discarded", code: "DISCARDED" }]);
    await expect(p).rejects.toMatchObject({ code: "DISCARDED" });
    expect(follower.pendingCount()).toBe(0);
    expect(follower.store.get("Note", "n0")).toBeNull();
  });

  test("a token change whose session refresh fails retries until the writes push", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    const inner = globalThis.fetch;
    let failMe = true;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      if (failMe && String(input).includes("/api/auth/me")) throw new TypeError("network down");
      return inner(input, init);
    }) as typeof fetch;
    env.signIn({ userId: "u1" }); // token rotation
    await env.engine.pull();
    await env.flush(20);
    const id = await env.engine.insert("Note", { title: "after rotation" });
    expect(env.engine.pendingCount()).toBe(1);
    failMe = false;
    for (let i = 0; i < 60 && env.engine.pendingCount() > 0; i++) {
      await new Promise((r) => setTimeout(r, 50));
    }
    expect(env.engine.pendingCount()).toBe(0);
    expect(env.server.receivedPushKeys).toContain(`Note/${id}`);
  }, 10_000);

  test("after a reload while signed out, the previous user's queued writes are not pushed", async () => {
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/sync/push")) throw new Error("must not push");
      const body = url.includes("/api/sync/pull")
        ? { changes: [], cursor: { last_seq: 0 }, has_more: false }
        : url.includes("/api/auth/me")
          ? { user_id: null }
          : {};
      return new Response(JSON.stringify(body), { status: 200 });
    }) as typeof fetch;
    const name = `owner-${Math.random().toString(36).slice(2)}`;
    // What the previous run left on disk: the replica tagged anonymous
    // (u1's session expired) and u1's queued write.
    const disk = new IndexedDBPersistence(name);
    await disk.open();
    await disk.saveIdentity(null);
    await disk.saveCursor({ last_seq: 1 });
    const { IndexedDBMutationPersistence } = await import("./persistence");
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
    expect(engine.pendingCount()).toBe(1);
    expect(engine.mutations.pending()[0]!.owner).toBe("u1");
    engine.stop();
    reopened.connection?.close();
  });

  test("a follower applies the leader's rows that arrive while its own reset runs", async () => {
    const persistence = new IndexedDBPersistence(`fence-${Math.random().toString(36).slice(2)}`);
    await persistence.open();
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persistence, multiTab: false });
    const internals = engine as unknown as {
      persistence: IndexedDBPersistence;
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
      multiTabHooks: () => {
        onResetReceived: (wipe: boolean) => void;
        onAppliedReceived: (c: ChangeEvent[], t?: { last_seq: number }) => void;
      };
    };
    internals.persistence = persistence;
    internals.isMultiTabLeader = false;
    internals.broadcastToTabs = () => {};
    const hooks = internals.multiTabHooks();
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    let started = false;
    const realClear = persistence.clear.bind(persistence);
    persistence.clear = async () => {
      started = true;
      await gate;
      return realClear();
    };
    hooks.onResetReceived(false);
    while (!started) await new Promise((r) => setTimeout(r, 1));
    hooks.onAppliedReceived(
      [{ seq: 3, entity: "Deal", row_id: "d1", kind: "insert", data: { id: "d1" }, timestamp: "" }],
      { last_seq: 3 },
    );
    release();
    await new Promise((r) => setTimeout(r, 50));
    expect(engine.store.list("Deal")).toHaveLength(1);
    expect(engine.getCursor().last_seq).toBe(3);
    engine.stop();
    persistence.connection?.close();
  });

  test("an update from a follower tab keeps its session owner when forwarded", async () => {
    const engine = new SyncEngine({ baseUrl: "http://test.invalid", persist: false, multiTab: false });
    const internals = engine as unknown as {
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
      multiTabHooks: () => { onSessionReceived: (s: unknown) => void };
      sessionChain: Promise<void>;
    };
    internals.isMultiTabLeader = false;
    const sent: { type?: string; ops?: PendingMutation[] }[] = [];
    internals.broadcastToTabs = (p) => sent.push(p as { type?: string });
    internals.multiTabHooks().onSessionReceived(session("uA"));
    await internals.sessionChain;
    void engine.insert("Note", { title: "draft" });
    await new Promise((r) => setTimeout(r, 0));
    const fwd = sent.find((m) => m.type === "mutations");
    expect(fwd?.ops?.[0]?.owner).toBe("uA");
  });
});
