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

describe("RoomSubscriptions: broadcast message channel", () => {
  test("registerMessages receives relayed broadcast payloads with sender", () => {
    const h = makeHarness();
    const received: unknown[] = [];
    h.rooms.registerMessages("battle", (m) => received.push(m));
    // First message-subscriber ships the wire subscribe — broadcasts
    // arrive on the same room-subscribe as membership.
    expect(h.sent).toEqual([{ type: "room-subscribe", room: "battle" }]);

    h.rooms.applyUpdate(
      "battle",
      "broadcast",
      { user_id: "u_42", joined_at: "", data: {} },
      { topic: "fire", payload: { k: "s" } },
    );
    expect(received).toEqual([{ topic: "fire", payload: { k: "s" }, from: "u_42" }]);
  });

  test("broadcasts do NOT pulse membership subscribers (fire-rate traffic)", () => {
    const h = makeHarness();
    let membershipPulses = 0;
    h.rooms.register("battle", () => membershipPulses++);
    h.rooms.applySnapshot("battle", []);
    const after = membershipPulses;
    h.rooms.applyUpdate("battle", "broadcast", { user_id: "u", joined_at: "", data: {} }, {
      topic: "fire",
      payload: 1,
    });
    expect(membershipPulses).toBe(after);
  });

  test("message-only subscriber refcounts the wire sub like membership", () => {
    const h = makeHarness();
    const off = h.rooms.registerMessages("battle", () => {});
    expect(h.sent).toEqual([{ type: "room-subscribe", room: "battle" }]);
    off();
    expect(h.sent).toEqual([
      { type: "room-subscribe", room: "battle" },
      { type: "room-unsubscribe", room: "battle" },
    ]);
    // Double-unsubscribe is harmless.
    off();
    expect(h.sent.length).toBe(2);
  });

  test("mixed membership + message subscribers share one wire sub", () => {
    const h = makeHarness();
    const offMembers = h.rooms.register("battle", () => {});
    const offMessages = h.rooms.registerMessages("battle", () => {});
    expect(h.sent.length).toBe(1); // one subscribe for both
    offMembers();
    expect(h.sent.length).toBe(1); // message sub still holds the room
    offMessages();
    expect(h.sent).toEqual([
      { type: "room-subscribe", room: "battle" },
      { type: "room-unsubscribe", room: "battle" },
    ]);
  });
});

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

  test("applyUpdate presence applies the envelope's data slot", () => {
    // The server ships new presence in the DATA slot; `member` carries
    // only user_id. Pre-fix the handler merged `member` alone — a no-op
    // that froze remote presence (live cursors, typing indicators) at
    // whatever the join carried.
    const h = makeHarness();
    h.rooms.register("channel:foo", () => {});
    h.rooms.applySnapshot("channel:foo", [
      { user_id: "u1", joined_at: "t1", data: { caret: 1 } },
    ]);
    h.rooms.applyUpdate(
      "channel:foo",
      "presence",
      { user_id: "u1", joined_at: "t1" },
      { caret: 42, name: "Ada" },
    );
    const members = h.rooms.members("channel:foo")!;
    expect(members[0].data).toEqual({ caret: 42, name: "Ada" });
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
