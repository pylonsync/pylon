// Unit tests for ServerSubscriptions. Pins the replay-on-reconnect
// contract so both consumers (CRDT + reactive) get identical
// semantics from one registry.

import { describe, expect, test } from "bun:test";

import { ServerSubscriptions } from "./server-subscriptions";

describe("ServerSubscriptions", () => {
  test("register sends the subscribe message", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("k1", { type: "sub", k: "k1" });
    expect(sent).toEqual([{ type: "sub", k: "k1" }]);
  });

  test("re-registering the same key with identical payload does NOT re-send", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("k1", { type: "sub", k: "k1", v: 1 });
    subs.register("k1", { type: "sub", k: "k1", v: 1 });
    expect(sent.length).toBe(1);
  });

  test("re-registering with a CHANGED payload re-sends", () => {
    // The intended path for useReactiveQuery(name, args) when args
    // change: same sub_id, different args; the server has to learn.
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("k1", { type: "sub", k: "k1", v: 1 });
    subs.register("k1", { type: "sub", k: "k1", v: 2 });
    expect(sent).toEqual([
      { type: "sub", k: "k1", v: 1 },
      { type: "sub", k: "k1", v: 2 },
    ]);
    // Most recent payload wins on replay.
    sent.length = 0;
    subs.replay();
    expect(sent).toEqual([{ type: "sub", k: "k1", v: 2 }]);
  });

  test("payload equality is structural (key order, nested)", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("k1", { type: "sub", args: { a: 1, b: 2 } });
    // Same logical payload, different key insertion order.
    subs.register("k1", { type: "sub", args: { b: 2, a: 1 } });
    expect(sent.length).toBe(1);
  });

  test("unregister sends the unsubscribe and forgets the spec", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("k1", { type: "sub", k: "k1" });
    subs.unregister("k1", { type: "unsub", k: "k1" });
    expect(sent).toEqual([
      { type: "sub", k: "k1" },
      { type: "unsub", k: "k1" },
    ]);
    expect(subs.has("k1")).toBe(false);
    // Replay after unregister sends nothing.
    sent.length = 0;
    subs.replay();
    expect(sent).toEqual([]);
  });

  test("unregister of an unknown key is a no-op", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.unregister("ghost", { type: "unsub", k: "ghost" });
    expect(sent).toEqual([]);
  });

  test("replay re-sends every active subscribe message in order", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("a", { type: "sub", k: "a" });
    subs.register("b", { type: "sub", k: "b" });
    subs.register("c", { type: "sub", k: "c" });
    sent.length = 0;
    subs.replay();
    expect(sent).toEqual([
      { type: "sub", k: "a" },
      { type: "sub", k: "b" },
      { type: "sub", k: "c" },
    ]);
  });

  test("replay skips unregistered keys", () => {
    const sent: unknown[] = [];
    const subs = new ServerSubscriptions((m) => sent.push(m));
    subs.register("a", { type: "sub", k: "a" });
    subs.register("b", { type: "sub", k: "b" });
    subs.unregister("a", { type: "unsub", k: "a" });
    sent.length = 0;
    subs.replay();
    expect(sent).toEqual([{ type: "sub", k: "b" }]);
  });
});
