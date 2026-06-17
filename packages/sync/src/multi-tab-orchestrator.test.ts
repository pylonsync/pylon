// Unit tests for MultiTabOrchestrator. Drives the dispatch table by
// calling handleMessage() directly — same path the BroadcastChannel's
// onmessage hits in production. The broker itself is stubbed so the
// election + heartbeat machinery doesn't fire during these tests.

import { describe, expect, test } from "bun:test";

import { MultiTabOrchestrator } from "./multi-tab-orchestrator";
import { ServerSubscriptions } from "./server-subscriptions";
import { SubscriptionCoordinator } from "./subscription-coordinator";
import type { ChangeEvent, ResolvedSession } from "./types";

function makeRig(opts: { isLeader?: boolean } = {}) {
  const events: { kind: string; args: unknown[] }[] = [];
  const record = (kind: string) =>
    (...args: unknown[]) => {
      events.push({ kind, args });
    };
  const serverSubs = new ServerSubscriptions(() => {});
  const subs = new SubscriptionCoordinator(serverSubs, {
    isLeader: () => opts.isLeader ?? true,
    broadcastToTabs: () => {},
  });
  const orch = new MultiTabOrchestrator(
    { enabled: false, appName: "test" },
    subs,
    {
      onInitialLeader: record("initialLeader"),
      onLatePromote: record("latePromote"),
      onDemote: record("demote"),
      onAppliedReceived: record("applied"),
      onReconciledReceived: record("reconciled"),
      onResetReceived: record("reset"),
      onSessionReceived: record("session"),
      onMutationsForwarded: record("mutationsForwarded"),
      onMutationsAcked: record("mutationsAcked"),
      onMutationsFailed: record("mutationsFailed"),
      onBinaryReceived: record("binary"),
      onPeerLeft: record("peerLeft"),
      onReplayForwardedMutations: record("replayForwardedMutations"),
    },
  );
  return { orch, events, serverSubs, subs };
}

describe("MultiTabOrchestrator dispatch", () => {
  test("applied envelope fires onAppliedReceived with changes + cursor", () => {
    const { orch, events } = makeRig();
    const change: ChangeEvent = {
      seq: 7,
      entity: "Todo",
      row_id: "r1",
      kind: "insert",
      data: { id: "r1" },
      timestamp: "",
    };
    orch.handleMessage(
      { type: "applied", changes: [change], targetCursor: { last_seq: 7 } },
      "leader-x",
    );
    expect(events.length).toBe(1);
    expect(events[0].kind).toBe("applied");
    const [changes, cursor] = events[0].args as [ChangeEvent[], { last_seq: number }];
    expect(changes[0].seq).toBe(7);
    expect(cursor.last_seq).toBe(7);
  });

  test("session envelope fires onSessionReceived", () => {
    const { orch, events } = makeRig();
    const resolved: ResolvedSession = {
      userId: "u1",
      tenantId: "org-a",
      isAdmin: false,
      roles: [],
    };
    orch.handleMessage({ type: "session", resolved }, "leader-x");
    expect(events.length).toBe(1);
    expect(events[0].kind).toBe("session");
    expect(events[0].args[0]).toEqual(resolved);
  });

  test("mutations envelope only fires onMutationsForwarded on the leader", () => {
    const follower = makeRig({ isLeader: false });
    follower.orch.handleMessage(
      { type: "mutations", ops: [] },
      "follower-x",
    );
    // No event because this rig isn't leader. The orchestrator's
    // leader gate filtered it out.
    expect(follower.events.length).toBe(0);
  });

  test("request-sub-replay re-forwards a follower's pending mutations (#341)", () => {
    // A new leader broadcasts request-sub-replay after a handoff. Followers
    // must re-forward their pending mutations — an op forwarded to the
    // previous (now-dead) leader is otherwise stranded, since the new leader
    // only drains its OWN queue on promotion. makeRig's orchestrator never
    // ran init(), so _isLeader is false → it acts as a follower here.
    const { orch, events } = makeRig();
    orch.handleMessage({ type: "request-sub-replay" }, "new-leader");
    expect(events.map((e) => e.kind)).toContain("replayForwardedMutations");
  });

  test("request-sub-replay does NOT re-forward on the leader (#341)", () => {
    const { orch, events } = makeRig();
    // Promote this rig to leader; a leader receiving the replay request (its
    // own echo, or a stale broadcast) owns the network and must not act.
    (orch as unknown as { _isLeader: boolean })._isLeader = true;
    orch.handleMessage({ type: "request-sub-replay" }, "x");
    expect(events.map((e) => e.kind)).not.toContain("replayForwardedMutations");
  });

  test("mutations-acked + mutations-failed both fire their hooks", () => {
    const { orch, events } = makeRig();
    orch.handleMessage({ type: "mutations-acked", opIds: ["a", "b"] }, "x");
    orch.handleMessage(
      { type: "mutations-failed", ops: [{ opId: "c", error: "bad" }] },
      "x",
    );
    expect(events.map((e) => e.kind)).toEqual([
      "mutationsAcked",
      "mutationsFailed",
    ]);
    expect((events[0].args[0] as string[])[0]).toBe("a");
    expect((events[1].args[0] as { opId: string; error: string }[])[0].opId).toBe(
      "c",
    );
  });

  test("sub-register and sub-unregister route directly to SubscriptionCoordinator", async () => {
    const { orch, serverSubs, events } = makeRig();
    // Sub-register / sub-unregister are leader-gated inside the
    // orchestrator (followers can't act on a peer's request). Init
    // the orchestrator so it observes itself as leader.
    await orch.init();
    events.length = 0; // drop the initialLeader event from init
    orch.handleMessage(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
      "follower-1",
    );
    expect(serverSubs.has("Todo\x00r1")).toBe(true);
    // Engine hooks were NOT fired — subscription dispatch bypasses them.
    expect(events.length).toBe(0);
    orch.handleMessage(
      {
        type: "sub-unregister",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
      "follower-1",
    );
    expect(serverSubs.has("Todo\x00r1")).toBe(false);
  });

  test("binary envelope fires onBinaryReceived with the bytes", () => {
    const { orch, events } = makeRig();
    const bytes = new Uint8Array([1, 2, 3]);
    orch.handleMessage({ type: "binary", bytes }, "leader-x");
    expect(events.length).toBe(1);
    expect(events[0].kind).toBe("binary");
    expect((events[0].args[0] as Uint8Array)[0]).toBe(1);
  });

  test("reset envelope fires onResetReceived", () => {
    const { orch, events } = makeRig();
    orch.handleMessage({ type: "reset" }, "leader-x");
    expect(events.map((e) => e.kind)).toEqual(["reset"]);
  });

  test("unknown envelope is a silent no-op", () => {
    const { orch, events } = makeRig();
    expect(() =>
      orch.handleMessage({ type: "bogus-type-no-one-knows" }, "x"),
    ).not.toThrow();
    expect(events.length).toBe(0);
  });
});

describe("MultiTabOrchestrator init", () => {
  test("multiTab:false short-circuits to sole-leader", async () => {
    const { orch, events } = makeRig();
    const leader = await orch.init();
    expect(leader).toBe(true);
    expect(events.map((e) => e.kind)).toContain("initialLeader");
  });
});
