// Unit tests for RoomSubscriptions. Pins the contract the engine
// depends on:
//   - register/unregister refcount → first add sends room-subscribe,
//     last remove sends room-unsubscribe
//   - applySnapshot / applyUpdate / applyError mutate cached state and
//     pulse subscribers
//   - replay() resends room-subscribe for every active room (used by
//     the engine's onConnected hook so reconnect resyncs membership)
//   - sendWs returning false (no WS) doesn't break refcount semantics
//
// The engine integration (multi-tab forwarding, fanout) is covered in
// the scenarios suite — these tests pin the registry in isolation so
// regressions surface at the layer they originated.

import { describe, expect, test } from "bun:test";

import { RoomSubscriptions } from "./room-subscriptions";

function makeHarness() {
  const sent: unknown[] = [];
  // Default to "ws is open" so the refcount path actually ships
  // subscribe / unsubscribe frames; individual tests override.
  let wsOpen = true;
  const rooms = new RoomSubscriptions((msg) => {
    sent.push(msg);
    return wsOpen;
  });
  return {
    rooms,
    sent,
    setWsOpen(v: boolean) {
      wsOpen = v;
    },
  };
}

describe("RoomSubscriptions: refcount + wire frames", () => {
  test("first register ships room-subscribe; second is a no-op", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    expect(h.sent).toEqual([{ type: "room-subscribe", room: "channel:foo" }]);
    h.rooms.register("channel:foo", () => {});
    expect(h.sent.length).toBe(1); // no second subscribe
  });

  test("last unregister ships room-unsubscribe; mid-stream calls are no-ops on wire", () => {
    const h = makeHarness();
    const unsub1 = h.rooms.register("channel:foo", () => {});
    const unsub2 = h.rooms.register("channel:foo", () => {});
    unsub1();
    expect(h.sent.length).toBe(1); // still just the initial subscribe
    unsub2();
    expect(h.sent).toEqual([
      { type: "room-subscribe", room: "channel:foo" },
      { type: "room-unsubscribe", room: "channel:foo" },
    ]);
  });

  test("unregisterRoom force-tears-down regardless of refcount", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    h.rooms.register("channel:foo", () => {});
    h.rooms.unregisterRoom("channel:foo");
    expect(h.sent).toEqual([
      { type: "room-subscribe", room: "channel:foo" },
      { type: "room-unsubscribe", room: "channel:foo" },
    ]);
    expect(h.rooms.has("channel:foo")).toBe(false);
  });

  test("sendWs returning false (WS closed / follower) still refcounts correctly", () => {
    const h = makeHarness();
    h.setWsOpen(false);
    const unsub = h.rooms.register("channel:foo", () => {});
    // The send happened (registry doesn't care if it landed); it just
    // didn't reach the wire. Engine's replay() on reconnect handles the
    // catch-up.
    expect(h.sent.length).toBe(1);
    unsub();
    expect(h.sent.length).toBe(2); // unsubscribe still queued
  });
});

describe("RoomSubscriptions: snapshot / update / error", () => {
  test("applySnapshot stores members and pulses every subscriber", () => {
    const h = makeHarness();
    let calls = 0;
    h.rooms.register("channel:foo", () => {
      calls += 1;
    });
    h.rooms.applySnapshot("channel:foo", [
      { user_id: "u1", joined_at: "t1" },
      { user_id: "u2", joined_at: "t2" },
    ]);
    expect(h.rooms.members("channel:foo")).toHaveLength(2);
    expect(calls).toBe(1);
  });

  test("members() returns null before any snapshot lands", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    expect(h.rooms.members("channel:foo")).toBeNull();
  });

  test("applyUpdate join inserts a member; idempotent on duplicate user_id", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    h.rooms.applySnapshot("channel:foo", []);
    h.rooms.applyUpdate(
      "channel:foo",
      "join",
      { user_id: "u1", joined_at: "t1" },
      undefined,
    );
    h.rooms.applyUpdate(
      "channel:foo",
      "join",
      { user_id: "u1", joined_at: "t1" },
      undefined,
    );
    expect(h.rooms.members("channel:foo")).toHaveLength(1);
  });

  test("applyUpdate leave removes the matching user", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    h.rooms.applySnapshot("channel:foo", [
      { user_id: "u1", joined_at: "t1" },
      { user_id: "u2", joined_at: "t2" },
    ]);
    h.rooms.applyUpdate(
      "channel:foo",
      "leave",
      { user_id: "u1", joined_at: "t1" },
      undefined,
    );
    expect(h.rooms.members("channel:foo")).toEqual([
      { user_id: "u2", joined_at: "t2" },
    ]);
  });

  test("applyError stores the error code and surfaces via error()", () => {
    const h = makeHarness();
    let calls = 0;
    h.rooms.register("channel:foo", () => {
      calls += 1;
    });
    h.rooms.applyError("channel:foo", { code: "NOT_IN_ROOM" });
    expect(h.rooms.error("channel:foo")).toEqual({ code: "NOT_IN_ROOM" });
    expect(calls).toBe(1);
  });

  test("late register on an already-snapshotted room pulses the new sub immediately", () => {
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    h.rooms.applySnapshot("channel:foo", [
      { user_id: "u1", joined_at: "t1" },
    ]);
    let latePulseCount = 0;
    h.rooms.register("channel:foo", () => {
      latePulseCount += 1;
    });
    expect(latePulseCount).toBe(1); // fired synchronously
  });
});

describe("RoomSubscriptions: replay on reconnect", () => {
  test("replay() resends room-subscribe for every active room", () => {
    const h = makeHarness();
    h.rooms.register("channel:a", () => {});
    h.rooms.register("channel:b", () => {});
    h.sent.length = 0;
    h.rooms.replay();
    expect(h.sent).toEqual([
      { type: "room-subscribe", room: "channel:a" },
      { type: "room-subscribe", room: "channel:b" },
    ]);
  });

  test("replay() skips fully-unregistered rooms", () => {
    const h = makeHarness();
    const unsub = h.rooms.register("channel:a", () => {});
    h.rooms.register("channel:b", () => {});
    unsub();
    h.sent.length = 0;
    h.rooms.replay();
    expect(h.sent).toEqual([{ type: "room-subscribe", room: "channel:b" }]);
  });
});
