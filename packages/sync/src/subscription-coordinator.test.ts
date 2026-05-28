// Unit tests for the SubscriptionCoordinator in isolation. Engine
// integration is covered by `round6-codex.test.ts` and the scenarios
// suite — these tests just pin the contract:
//  - CRDT add/remove is refcounted; only the first add + last remove
//    ship a WS frame
//  - leader path registers via serverSubs; follower path broadcasts
//  - forwarded sub-register / sub-unregister update the leader's
//    forwarder + owner sets
//  - scrubPeer unregisters when the last owner leaves

import { beforeEach, describe, expect, test } from "bun:test";

import { ServerSubscriptions } from "./server-subscriptions";
import { SubscriptionCoordinator } from "./subscription-coordinator";

function makeHarness(initialLeader = true) {
  let isLeader = initialLeader;
  const wsSent: unknown[] = [];
  const broadcasts: unknown[] = [];
  const serverSubs = new ServerSubscriptions((msg) => wsSent.push(msg));
  const coord = new SubscriptionCoordinator(serverSubs, {
    isLeader: () => isLeader,
    broadcastToTabs: (payload) => broadcasts.push(payload),
  });
  return {
    coord,
    serverSubs,
    wsSent,
    broadcasts,
    setLeader(v: boolean) {
      isLeader = v;
    },
  };
}

describe("SubscriptionCoordinator CRDT subs", () => {
  test("leader: first subscribeCrdt registers WS sub; second is a no-op", () => {
    const h = makeHarness(true);
    h.coord.subscribeCrdt("Todo", "r1");
    expect(h.serverSubs.has("Todo\x00r1")).toBe(true);
    const before = h.wsSent.length;
    h.coord.subscribeCrdt("Todo", "r1"); // refcount → 2
    expect(h.wsSent.length).toBe(before); // no new WS frame
  });

  test("leader: unsubscribe matches refcount; last call ships WS unsubscribe", () => {
    const h = makeHarness(true);
    h.coord.subscribeCrdt("Todo", "r1");
    h.coord.subscribeCrdt("Todo", "r1");
    h.coord.unsubscribeCrdt("Todo", "r1"); // refcount 2 → 1
    expect(h.serverSubs.has("Todo\x00r1")).toBe(true);
    h.coord.unsubscribeCrdt("Todo", "r1"); // refcount 1 → 0
    expect(h.serverSubs.has("Todo\x00r1")).toBe(false);
  });

  test("unsubscribe more times than subscribe is a no-op", () => {
    const h = makeHarness(true);
    expect(() => h.coord.unsubscribeCrdt("Todo", "ghost")).not.toThrow();
  });

  test("follower: subscribeCrdt broadcasts sub-register; no serverSubs entry", () => {
    const h = makeHarness(false);
    h.coord.subscribeCrdt("Todo", "r1");
    expect(h.serverSubs.has("Todo\x00r1")).toBe(false);
    expect(h.broadcasts).toEqual([
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
    ]);
  });

  test("leader keeps WS sub alive while a follower still has it forwarded", () => {
    const h = makeHarness(true);
    h.coord.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
      "follower-a",
    );
    expect(h.serverSubs.has("Todo\x00r1")).toBe(true);
    // Leader's own subscribe doesn't re-send (already alive via fwd).
    const before = h.wsSent.length;
    h.coord.subscribeCrdt("Todo", "r1");
    expect(h.wsSent.length).toBe(before);
    // Leader unsubscribe with follower still wanting it keeps the WS sub alive.
    h.coord.unsubscribeCrdt("Todo", "r1");
    expect(h.serverSubs.has("Todo\x00r1")).toBe(true);
  });
});

describe("SubscriptionCoordinator reactive subs", () => {
  test("leader: subscribeReactive registers + tracks self in owner set", () => {
    const h = makeHarness(true);
    h.coord.subscribeReactive("sub-1", "myFn", { v: 1 }, () => {});
    expect(h.serverSubs.has("sub-1")).toBe(true);
  });

  test("leader: unsubscribeReactive drops self; only unregisters when no other owner", () => {
    const h = makeHarness(true);
    h.coord.subscribeReactive("sub-1", "myFn", { v: 1 }, () => {});
    // Follower also forwarded the same sub_id (different mounts in different tabs).
    h.coord.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "reactive",
        key: "sub-1",
        sub_id: "sub-1",
        fn_name: "myFn",
        args: { v: 1 },
      },
      "follower-a",
    );
    h.coord.unsubscribeReactive("sub-1");
    // Follower still owns it → still registered.
    expect(h.serverSubs.has("sub-1")).toBe(true);
    h.coord.handleForwardedUnregister(
      {
        type: "sub-unregister",
        kind: "reactive",
        key: "sub-1",
        sub_id: "sub-1",
      },
      "follower-a",
    );
    // Now last owner left → unregistered.
    expect(h.serverSubs.has("sub-1")).toBe(false);
  });

  test("inbound reactive-msg routes to the local handler", () => {
    const h = makeHarness(true);
    const received: unknown[] = [];
    h.coord.subscribeReactive("sub-1", "fn", { v: 1 }, (msg) =>
      received.push(msg),
    );
    h.coord.handleReactiveMessage("sub-1", { kind: "result", result: 42 });
    expect(received).toEqual([{ kind: "result", result: 42 }]);
  });

  test("follower: subscribeReactive broadcasts sub-register", () => {
    const h = makeHarness(false);
    h.coord.subscribeReactive("sub-1", "fn", { v: 1 }, () => {});
    expect(h.broadcasts).toEqual([
      {
        type: "sub-register",
        kind: "reactive",
        key: "sub-1",
        sub_id: "sub-1",
        fn_name: "fn",
        args: { v: 1 },
      },
    ]);
  });
});

describe("SubscriptionCoordinator scrubPeer + replays", () => {
  test("scrubPeer removes a follower's CRDT entries and unregisters when last", () => {
    const h = makeHarness(true);
    h.coord.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
      "follower-a",
    );
    expect(h.serverSubs.has("Todo\x00r1")).toBe(true);
    h.coord.scrubPeer("follower-a");
    expect(h.serverSubs.has("Todo\x00r1")).toBe(false);
  });

  test("seedFromLocalInterest re-registers wanted subs on the new leader", () => {
    const h = makeHarness(false);
    // Mount a reactive sub as follower (broadcasts only).
    h.coord.subscribeReactive("sub-1", "fn", { v: 1 }, () => {});
    expect(h.serverSubs.has("sub-1")).toBe(false);
    // Promotion: now we're leader; seed should register via serverSubs.
    h.setLeader(true);
    h.coord.seedFromLocalInterest();
    expect(h.serverSubs.has("sub-1")).toBe(true);
  });

  test("hasCrdtForwarders reflects forwarder presence", () => {
    const h = makeHarness(true);
    expect(h.coord.hasCrdtForwarders()).toBe(false);
    h.coord.handleForwardedRegister(
      {
        type: "sub-register",
        kind: "crdt",
        key: "Todo\x00r1",
        entity: "Todo",
        rowId: "r1",
      },
      "follower-a",
    );
    expect(h.coord.hasCrdtForwarders()).toBe(true);
    h.coord.scrubPeer("follower-a");
    expect(h.coord.hasCrdtForwarders()).toBe(false);
  });
});
