// Integration tests for the engine's WS room subscription path.
// Exercises subscribeRoom / unsubscribeRoom against the in-process
// TestServer:
//   - subscribeRoom emits `room-subscribe` over the WS exactly once,
//     refcounted across multiple subscribers
//   - inbound `room-snapshot` populates the local registry + fires
//     subscriber callbacks
//   - inbound `room-update` action:join mutates the cached members
//   - WS reconnect resends `room-subscribe` for every active room
//   - last unsubscribe emits `room-unsubscribe`
//   - inbound `error { code: NOT_IN_ROOM, room }` surfaces via the
//     registry's error slot
//
// All harness-level — no real network. The TestServer's `pushToUser`
// + WS subprotocol bearer threading do the work.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { createTestEnv, type TestEnv } from "./test-harness";

function isRoomSubscribe(msg: unknown, room: string): boolean {
  return (
    typeof msg === "object" &&
    msg !== null &&
    (msg as Record<string, unknown>).type === "room-subscribe" &&
    (msg as Record<string, unknown>).room === room
  );
}

function isRoomUnsubscribe(msg: unknown, room: string): boolean {
  return (
    typeof msg === "object" &&
    msg !== null &&
    (msg as Record<string, unknown>).type === "room-unsubscribe" &&
    (msg as Record<string, unknown>).room === room
  );
}

function countWs(env: TestEnv, predicate: (msg: unknown) => boolean): number {
  return env.server.receivedWsMessages.filter((m) => predicate(m.msg)).length;
}

describe("engine WS room subscriptions", () => {
  let env: TestEnv;

  beforeEach(() => {
    env = createTestEnv();
  });

  afterEach(async () => {
    await env.dispose();
  });

  test("first subscribeRoom emits room-subscribe; second is dedup'd", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    env.engine.subscribeRoom("channel:foo", () => {});
    env.engine.subscribeRoom("channel:foo", () => {});
    await env.flush();
    expect(countWs(env, (m) => isRoomSubscribe(m, "channel:foo"))).toBe(1);
  });

  test("inbound room-snapshot fires subscriber callback with members", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    let callbackCount = 0;
    env.engine.subscribeRoom("channel:foo", () => {
      callbackCount += 1;
    });
    await env.flush();
    env.server.pushToUser("u1", {
      type: "room-snapshot",
      room: "channel:foo",
      members: [
        { user_id: "alice", joined_at: "t0" },
        { user_id: "bob", joined_at: "t1" },
      ],
    });
    await env.flush();
    expect(callbackCount).toBeGreaterThanOrEqual(1);
    const members = env.engine.getRoomMembers("channel:foo");
    expect(members).toHaveLength(2);
  });

  test("inbound room-update action:join mutates cached members", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    env.engine.subscribeRoom("channel:foo", () => {});
    await env.flush();
    env.server.pushToUser("u1", {
      type: "room-snapshot",
      room: "channel:foo",
      members: [{ user_id: "alice", joined_at: "t0" }],
    });
    env.server.pushToUser("u1", {
      type: "room-update",
      room: "channel:foo",
      action: "join",
      member: { user_id: "bob", joined_at: "t1" },
    });
    await env.flush();
    const members = env.engine.getRoomMembers("channel:foo");
    expect(members?.map((m) => m.user_id).sort()).toEqual(["alice", "bob"]);
  });

  test("last unsubscribeRoom emits room-unsubscribe + clears registry entry", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    const unsub1 = env.engine.subscribeRoom("channel:foo", () => {});
    const unsub2 = env.engine.subscribeRoom("channel:foo", () => {});
    await env.flush();
    unsub1();
    expect(countWs(env, (m) => isRoomUnsubscribe(m, "channel:foo"))).toBe(0);
    unsub2();
    await env.flush();
    expect(countWs(env, (m) => isRoomUnsubscribe(m, "channel:foo"))).toBe(1);
    expect(env.engine.getRoomMembers("channel:foo")).toBeNull();
  });

  test("inbound error { code: NOT_IN_ROOM } surfaces via getRoomError", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    let callbackFired = false;
    env.engine.subscribeRoom("channel:foo", () => {
      callbackFired = true;
    });
    await env.flush();
    env.server.pushToUser("u1", {
      type: "error",
      code: "NOT_IN_ROOM",
      room: "channel:foo",
      message: "not a member",
    });
    await env.flush();
    expect(callbackFired).toBe(true);
    expect(env.engine.getRoomError("channel:foo")).toEqual({
      code: "NOT_IN_ROOM",
      message: "not a member",
    });
  });

  test("isWebSocketConnected reflects WS open state", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    expect(env.engine.isWebSocketConnected()).toBe(true);
    expect(env.engine.getActiveTransportType()).toBe("websocket");
  });

  test("WS reconnect resends room-subscribe for every active room", async () => {
    // Tight reconnect budget so the test runs in a couple hundred ms
    // rather than seconds. Picks a short base delay; the full-jitter
    // formula caps the first attempt at random(0, 50ms).
    const reconnectEnv = createTestEnv({ reconnectDelay: 50 });
    try {
      reconnectEnv.signIn({ userId: "u1" });
      await reconnectEnv.start();
      reconnectEnv.engine.subscribeRoom("channel:a", () => {});
      reconnectEnv.engine.subscribeRoom("channel:b", () => {});
      await reconnectEnv.flush();
      expect(
        countWs(reconnectEnv, (m) => isRoomSubscribe(m, "channel:a")),
      ).toBe(1);
      expect(
        countWs(reconnectEnv, (m) => isRoomSubscribe(m, "channel:b")),
      ).toBe(1);
      // Force the WS to drop. The engine schedules a reconnect via its
      // backoff, opens a new socket, fires onConnected → rooms.replay().
      const closed = reconnectEnv.transport.closeLatestWs();
      expect(closed).toBe(true);
      await reconnectEnv.flush(200);
      expect(reconnectEnv.transport.wsConnectCount()).toBeGreaterThanOrEqual(2);
      // Replay: each active room got a fresh room-subscribe.
      expect(
        countWs(reconnectEnv, (m) => isRoomSubscribe(m, "channel:a")),
      ).toBe(2);
      expect(
        countWs(reconnectEnv, (m) => isRoomSubscribe(m, "channel:b")),
      ).toBe(2);
    } finally {
      await reconnectEnv.dispose();
    }
  });

  test("StrictMode-style double-subscribe to same room only ships one wire frame", async () => {
    env.signIn({ userId: "u1" });
    await env.start();
    // Simulate StrictMode: mount, mount again, unmount, unmount.
    const u1 = env.engine.subscribeRoom("channel:foo", () => {});
    const u2 = env.engine.subscribeRoom("channel:foo", () => {});
    await env.flush();
    expect(countWs(env, (m) => isRoomSubscribe(m, "channel:foo"))).toBe(1);
    u1();
    u2();
    await env.flush();
    expect(countWs(env, (m) => isRoomUnsubscribe(m, "channel:foo"))).toBe(1);
  });
});
