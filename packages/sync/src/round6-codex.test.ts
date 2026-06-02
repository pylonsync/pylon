// Regression tests for codex review round-6 findings on the
// multi-tab leader/follower integration glue in SyncEngine.
//
// Each test pins one finding so a future revert surfaces immediately:
//   1. `isMultiTabLeader` defaults to false (P1)
//   2. mutations-acked broadcast filters to actually-applied op_ids,
//      and mutations-failed carries the rest (P1)
//   3. A peer leaving (`bye` → onLeave) scrubs forwarded subs and
//      unregisters the WS subscription when no owner remains (P2)

import { afterEach, describe, expect, test } from "bun:test";

import { SyncEngine } from "./index";
import { createTestEnv, type TestEnv } from "./test-harness";

describe("codex round-6: isMultiTabLeader default", () => {
  test("a fresh engine WITHOUT multiTab:false is NOT leader", () => {
    // Pre-fix the default was `true`, so a tab that joined an
    // established election would never receive onPromote (it wasn't
    // promoted) or onDemote (it was never leader), and the engine's
    // leader-gated paths (WS, pull, push, poll) all ran on every tab.
    const engine = new SyncEngine({
      baseUrl: "http://stub.invalid",
      persist: false,
    });
    const internal = engine as unknown as { isMultiTabLeader: boolean };
    expect(internal.isMultiTabLeader).toBe(false);
  });

  test("multiTab:false in config promotes to leader from construction", () => {
    // Sanity for the constructor escape hatch: tests + apps that
    // explicitly disable multi-tab need to act as their own leader
    // immediately, before start().
    const engine = new SyncEngine({
      baseUrl: "http://stub.invalid",
      persist: false,
      multiTab: false,
    });
    const internal = engine as unknown as { isMultiTabLeader: boolean };
    expect(internal.isMultiTabLeader).toBe(true);
  });
});

describe("codex round-6: mutations-acked broadcast filters by status", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  test("only applied op_ids broadcast as acks; failed ones broadcast separately", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    const broadcasts: { type: string; payload: unknown }[] = [];
    const engine = env.engine as unknown as {
      broadcastToTabs(payload: unknown): void;
      request<T>(method: string, path: string, body?: unknown): Promise<T>;
      mutations: {
        add(change: unknown): string;
      };
      push(): Promise<void>;
    };

    // Spy on broadcastToTabs by replacing it. Save the original so
    // the engine's internal calls still hit the recorder.
    const originalBroadcast = engine.broadcastToTabs.bind(engine);
    engine.broadcastToTabs = (payload: unknown) => {
      const p = payload as { type: string };
      if (p.type === "mutations-acked" || p.type === "mutations-failed") {
        broadcasts.push({ type: p.type, payload });
      }
      originalBroadcast(payload);
    };

    // Stub the engine's HTTP request so /api/sync/push returns a
    // mixed-status response: op-good applied, op-bad error. Other
    // request paths fall through to the real harness fetch.
    const originalRequest = engine.request.bind(engine);
    engine.request = (async <T,>(
      method: string,
      path: string,
      body?: unknown,
    ): Promise<T> => {
      if (path === "/api/sync/push") {
        return {
          applied: 1,
          deduped: 0,
          errors: ["rejected by validation"],
          results: [
            { op_id: "op-good", status: "applied", seq: 1 },
            {
              op_id: "op-bad",
              status: "error",
              error: { code: "VALIDATION_ERROR", message: "rejected by validation" },
            },
          ],
          cursor: { last_seq: 1 },
        } as unknown as T;
      }
      return originalRequest(method, path, body);
    }) as typeof engine.request;

    // Two mutations, one good one bad. add() preserves op_id from
    // the change envelope (this is the multi-tab leader's behavior
    // when it forwards a follower's batch).
    engine.mutations.add({
      op_id: "op-good",
      entity: "Todo",
      row_id: "good",
      kind: "insert",
      data: { id: "good", text: "ok" },
    });
    engine.mutations.add({
      op_id: "op-bad",
      entity: "Todo",
      row_id: "bad",
      kind: "insert",
      data: { id: "bad", text: "boom" },
    });

    await engine.push();
    await env.flush();

    const acks = broadcasts.filter((b) => b.type === "mutations-acked");
    const fails = broadcasts.filter((b) => b.type === "mutations-failed");

    expect(acks.length).toBe(1);
    expect(fails.length).toBe(1);

    const ackedIds = (acks[0].payload as { opIds: string[] }).opIds;
    expect(ackedIds).toEqual(["op-good"]);

    const failedOps = (fails[0].payload as {
      ops: { opId: string; error: string }[];
    }).ops;
    expect(failedOps.length).toBe(1);
    expect(failedOps[0].opId).toBe("op-bad");
    expect(failedOps[0].error.length).toBeGreaterThan(0);
  });

  test("follower receiving mutations-failed marks the op failed (not applied)", async () => {
    // Receiving side: confirms the new follower handler doesn't lose
    // the failure. Pre-fix the leader sent every op_id as acked and
    // the follower silently clear()ed them; now the follower stays
    // failed so UI / retry can act.
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    const engine = env.engine as unknown as {
      handleMultiTabMessage(msg: unknown, from: string): void;
      mutations: {
        add(change: unknown): string;
        pending(): { id: string; status: string; error?: string }[];
      };
    };

    // Seed a pending mutation locally — simulates the follower's
    // optimistic queue.
    engine.mutations.add({
      op_id: "op-x",
      entity: "Todo",
      row_id: "x",
      kind: "insert",
      data: { id: "x", text: "y" },
    });

    // Leader → follower envelope. Drive through the orchestrator's
    // public message dispatch — same path the BroadcastChannel
    // onmessage hits in production.
    (engine as unknown as {
      orchestrator: {
        handleMessage(msg: unknown, from: string): void;
      };
    }).orchestrator.handleMessage(
      {
        type: "mutations-failed",
        ops: [{ opId: "op-x", error: "server rejected" }],
      },
      "leader-tab-id",
    );

    // The mutation stays in the queue, status=failed, with the error
    // string preserved.
    const all = (engine.mutations as unknown as {
      queue: { id: string; status: string; error?: string }[];
    }).queue;
    const found = all.find((m) => m.id === "op-x");
    expect(found).toBeDefined();
    expect(found?.status).toBe("failed");
    expect(found?.error).toBe("server rejected");
  });
});

describe("codex round-6: peer leaving scrubs forwarded subs", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  // Helper: reach into the SubscriptionCoordinator that the engine
  // delegates to. The state these tests pin lives on the coordinator
  // now, not the engine itself. The orchestrator's inbound dispatch
  // is bypassed by calling subscriptions.handleForwardedRegister /
  // scrubPeer directly — that's the same path the orchestrator takes
  // when a real broker message arrives, just driven by the test.
  function internals(env: TestEnv) {
    return env.engine as unknown as {
      serverSubs: { has(k: string): boolean };
      subscriptions: {
        crdtForwarders: Map<string, Set<string>>;
        reactiveSubOwners: Map<string, Set<string>>;
        handleForwardedRegister(msg: unknown, fromTabId: string): void;
        scrubPeer(tabId: string): void;
      };
    };
  }

  test("onMultiTabPeerLeft removes the tab from crdtForwarders + unregisters WS sub", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    const engine = internals(env);

    // Simulate a follower forwarding a CRDT sub via the broker
    // app-message path. The leader's handler creates an entry in
    // crdtForwarders and registers with serverSubs.
    engine.subscriptions.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00row-1",
        entity: "Todo",
        rowId: "row-1",
      },
      "follower-1",
    );

    expect(
      engine.subscriptions.crdtForwarders.get("Todo\x00row-1")?.has("follower-1"),
    ).toBe(true);
    expect(engine.serverSubs.has("Todo\x00row-1")).toBe(true);

    // Now the follower disappears (broker fires onLeave).
    engine.subscriptions.scrubPeer("follower-1");

    expect(engine.subscriptions.crdtForwarders.has("Todo\x00row-1")).toBe(false);
    expect(engine.serverSubs.has("Todo\x00row-1")).toBe(false);
  });

  test("onMultiTabPeerLeft keeps the sub alive if another tab still owns it", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    const engine = internals(env);

    // Two followers forward the SAME crdt key. The leader keeps a
    // single server sub with two entries in the forwarder set.
    engine.subscriptions.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00row-1",
        entity: "Todo",
        rowId: "row-1",
      },
      "follower-a",
    );
    engine.subscriptions.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00row-1",
        entity: "Todo",
        rowId: "row-1",
      },
      "follower-b",
    );

    expect(engine.subscriptions.crdtForwarders.get("Todo\x00row-1")?.size).toBe(2);
    expect(engine.serverSubs.has("Todo\x00row-1")).toBe(true);

    // Only follower-a leaves. The sub stays alive because follower-b
    // still owns it.
    engine.subscriptions.scrubPeer("follower-a");

    expect(engine.subscriptions.crdtForwarders.get("Todo\x00row-1")?.size).toBe(1);
    expect(
      engine.subscriptions.crdtForwarders.get("Todo\x00row-1")?.has("follower-b"),
    ).toBe(true);
    expect(engine.serverSubs.has("Todo\x00row-1")).toBe(true);
  });

  test("onMultiTabPeerLeft scrubs reactive owner sets and unregisters when empty", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    const engine = internals(env);

    engine.subscriptions.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "reactive",
        key: "sub-1",
        sub_id: "sub-1",
        fn_name: "q",
        args: { v: 1 },
      },
      "follower-x",
    );

    expect(engine.subscriptions.reactiveSubOwners.get("sub-1")?.has("follower-x")).toBe(
      true,
    );
    expect(engine.serverSubs.has("sub-1")).toBe(true);

    engine.subscriptions.scrubPeer("follower-x");

    expect(engine.subscriptions.reactiveSubOwners.has("sub-1")).toBe(false);
    expect(engine.serverSubs.has("sub-1")).toBe(false);
  });
});

describe("sync hardening: multi-tab follower coverage", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  // FOLLOWER GHOST ROLLBACK. The leader broadcasts `mutations-failed`;
  // the follower must roll back its OWN optimistic ghost, not just mark
  // the queue entry failed. Pre-fix onMutationsFailed called markFailed
  // directly, so the ghost row stuck around in the tab the user sees.
  test("a mutations-failed for an optimistic insert removes the follower's ghost", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    const engine = env.engine;

    // Optimistic ghost in the store + a matching queued mutation (as if
    // this follower had forwarded the insert to the leader).
    engine.store.optimisticInsertWithId("Todo", "t1", { id: "t1", text: "ghost" });
    engine.mutations.add({
      op_id: "op-1",
      entity: "Todo",
      row_id: "t1",
      kind: "insert",
      data: { id: "t1", text: "ghost" },
    });
    expect(engine.store.get("Todo", "t1")).not.toBeNull();

    (engine as unknown as {
      orchestrator: { handleMessage(msg: unknown, from: string): void };
    }).orchestrator.handleMessage(
      { type: "mutations-failed", ops: [{ opId: "op-1", error: "rejected" }] },
      "leader-tab",
    );

    // Ghost gone (rolled back), mutation marked failed.
    expect(engine.store.get("Todo", "t1")).toBeNull();
  });

  // ENTITY-OBSERVE FORWARDING. A leader that receives a forwarded
  // `entity-observe` from a follower must add the entity to its reconcile
  // sweep and fetch it — so a server row the follower never cached is
  // discovered + broadcast back. Without this a follower's useQuery on a
  // never-cached entity renders empty forever.
  test("a leader fetches an entity a follower forwarded via entity-observe", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start(); // solo → this engine is the leader
    const engine = env.engine;

    // A server row exists for an entity this engine never cached.
    env.server.insert("Domain", { id: "d1", host: "x.com" });
    expect(engine.store.list("Domain")).toHaveLength(0);

    (engine as unknown as {
      orchestrator: { handleMessage(msg: unknown, from: string): void };
    }).orchestrator.handleMessage(
      { type: "entity-observe", entity: "Domain" },
      "follower-tab",
    );
    await env.flush();

    expect(engine.store.get("Domain", "d1")).not.toBeNull();
  });

  // FORWARDED-MUTATION ROLLBACK (pins onMutationsForwarded prevRow
  // threading). When a follower forwards an update/delete and the leader's
  // push of it is PERMANENTLY rejected, the leader must restore the
  // canonical row — NOT delete it. Pre-fix the leader queued the forwarded
  // op via mutations.add(op.change) WITHOUT prevRow, so failPushedMutation
  // ran restoreRow(undefined ?? null) → restoreRow(null) → DELETED the
  // leader's still-valid canonical row. Data loss on every rejected
  // forwarded edit.
  test("a rejected forwarded UPDATE keeps the leader's canonical row", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "canonical" }]);
    await env.start(); // solo → leader, holds the canonical row
    await env.flush();
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();

    // A follower optimistically edited n1, captured the prior value as
    // prevRow, and forwarded the update. The leader's push will 403.
    env.server.primeNextPushOutcome({ kind: "status", status: 403 });
    (env.engine as unknown as {
      orchestrator: { handleMessage(msg: unknown, from: string): void };
    }).orchestrator.handleMessage(
      {
        type: "mutations",
        ops: [
          {
            id: "op-u",
            change: {
              op_id: "op-u",
              entity: "Note",
              row_id: "n1",
              kind: "update",
              data: { title: "edited" },
            },
            status: "pending",
            prevRow: { id: "n1", title: "canonical" },
          },
        ],
      },
      "follower-tab",
    );
    // Wait behind the forwarded push (opQueue serializes), then drain.
    await env.engine.push();
    await env.flush();

    const row = env.engine.store.get("Note", "n1") as { title?: string } | null;
    expect(row).not.toBeNull();
    expect(row?.title).toBe("canonical");
  });

  test("a rejected forwarded DELETE keeps the leader's canonical row", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "keep" }]);
    await env.start();
    await env.flush();
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();

    env.server.primeNextPushOutcome({ kind: "status", status: 403 });
    (env.engine as unknown as {
      orchestrator: { handleMessage(msg: unknown, from: string): void };
    }).orchestrator.handleMessage(
      {
        type: "mutations",
        ops: [
          {
            id: "op-d",
            change: {
              op_id: "op-d",
              entity: "Note",
              row_id: "n1",
              kind: "delete",
            },
            status: "pending",
            prevRow: { id: "n1", title: "keep" },
          },
        ],
      },
      "follower-tab",
    );
    await env.engine.push();
    await env.flush();

    // Pre-fix the leader deleted its canonical row on the rejected
    // forwarded delete; with prevRow threaded it survives.
    const row = env.engine.store.get("Note", "n1") as { title?: string } | null;
    expect(row).not.toBeNull();
    expect(row?.title).toBe("keep");
  });
});
