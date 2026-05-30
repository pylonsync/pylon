// useRoom — WebSocket push path coverage.
//
// The legacy polling-only path is covered in `useRoom.test.ts`; this
// file focuses on the v0.3.216 push protocol integration:
//
//   - WS connected + engine wired in → registry attaches to
//     engine.subscribeRoom AND does NOT start a polling interval
//   - inbound room-snapshot via the engine flows through to subscriber
//     callbacks
//   - inbound room-update action:join updates peers in the registry
//   - WS not connected → polling fallback fires (existing behaviour
//     preserved when the engine reports the WS down)
//   - StrictMode double-mount with WS → still one engine subscription
//
// We drive the registry directly via `__roomRegistryInternals` and feed
// in a controllable fake engine — no React renderer involved, no real
// network. The fake engine matches the surface the hook actually
// consumes (subscribeRoom, getRoomMembers, getRoomError,
// isWebSocketConnected, connectionStatus, store.subscribe).

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { __roomRegistryInternals } from "./useRoom";

// --- fetch stub (mirrors useRoom.test.ts) ------------------------------

interface RecordedRequest {
  url: string;
  method: string;
}

let recorded: RecordedRequest[];
let originalFetch: typeof globalThis.fetch;

function installFetchStub(): void {
  recorded = [];
  originalFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    recorded.push({ url, method });
    return new Response(JSON.stringify({ snapshot: { peers: [] }, members: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

function restoreFetch(): void {
  globalThis.fetch = originalFetch;
}

function isHeartbeat(r: RecordedRequest): boolean {
  return r.method === "GET" && /\/api\/rooms\/[^/]+$/.test(r.url);
}

// --- fake engine -------------------------------------------------------

interface FakeRoomEntry {
  callbacks: Set<() => void>;
  members: Array<{ user_id: string; joined_at: string; data?: any }> | null;
  error: { code: string; message?: string } | null;
}

interface FakeEngine {
  // Surface that useRoom actually pokes:
  subscribeRoom: (roomId: string, cb: () => void) => () => void;
  getRoomMembers: (roomId: string) =>
    | Array<{ user_id: string; joined_at: string; data?: any }>
    | null;
  getRoomError: (roomId: string) => { code: string; message?: string } | null;
  isWebSocketConnected: () => boolean;
  connectionStatus: () => string;
  // The store.subscribe surface — useRoom subscribes to connection-status
  // changes via it.
  store: {
    subscribe: (fn: () => void) => () => void;
  };

  // Test helpers — not on the real engine.
  _wsOpen: boolean;
  _statusListeners: Set<() => void>;
  _rooms: Map<string, FakeRoomEntry>;
  _subscribeCalls: string[];
  _unsubscribeCalls: string[];
  setWsConnected(open: boolean): void;
  pushSnapshot(roomId: string, members: any[]): void;
  pushUpdate(
    roomId: string,
    action: "join" | "leave" | "presence" | "broadcast",
    member?: any,
  ): void;
  pushError(roomId: string, code: string, message?: string): void;
}

function makeFakeEngine(): FakeEngine {
  const engine: FakeEngine = {
    _wsOpen: true,
    _statusListeners: new Set(),
    _rooms: new Map(),
    _subscribeCalls: [],
    _unsubscribeCalls: [],

    isWebSocketConnected: () => engine._wsOpen,
    connectionStatus: () => (engine._wsOpen ? "connected" : "reconnecting"),
    store: {
      subscribe: (fn) => {
        engine._statusListeners.add(fn);
        return () => engine._statusListeners.delete(fn);
      },
    },
    subscribeRoom: (roomId, cb) => {
      engine._subscribeCalls.push(roomId);
      let entry = engine._rooms.get(roomId);
      if (!entry) {
        entry = { callbacks: new Set(), members: null, error: null };
        engine._rooms.set(roomId, entry);
      }
      entry.callbacks.add(cb);
      // Fire one tick if there's already cached state (registry late-
      // subscriber path).
      if (entry.members !== null || entry.error !== null) cb();
      return () => {
        const e = engine._rooms.get(roomId);
        if (!e) return;
        e.callbacks.delete(cb);
        if (e.callbacks.size === 0) {
          engine._rooms.delete(roomId);
          engine._unsubscribeCalls.push(roomId);
        }
      };
    },
    getRoomMembers: (roomId) => engine._rooms.get(roomId)?.members ?? null,
    getRoomError: (roomId) => engine._rooms.get(roomId)?.error ?? null,

    setWsConnected: (open) => {
      engine._wsOpen = open;
      for (const fn of engine._statusListeners) fn();
    },
    pushSnapshot: (roomId, members) => {
      const entry = engine._rooms.get(roomId);
      if (!entry) return;
      entry.members = members;
      entry.error = null;
      for (const cb of entry.callbacks) cb();
    },
    pushUpdate: (roomId, action, member) => {
      const entry = engine._rooms.get(roomId);
      if (!entry) return;
      if (entry.members === null) entry.members = [];
      if (action === "join" && member) {
        const filtered = entry.members.filter(
          (m) => m.user_id !== member.user_id,
        );
        filtered.push(member);
        entry.members = filtered;
      } else if (action === "leave" && member) {
        entry.members = entry.members.filter(
          (m) => m.user_id !== member.user_id,
        );
      }
      for (const cb of entry.callbacks) cb();
    },
    pushError: (roomId, code, message) => {
      const entry = engine._rooms.get(roomId);
      if (!entry) return;
      entry.error = { code, message };
      for (const cb of entry.callbacks) cb();
    },
  };
  return engine;
}

// --- harness -----------------------------------------------------------

const BASE = "http://stub.invalid";
const ROOM = "channel:foo";
const USER = "user-1";
const TOKEN = "tok-1";

async function flush(ms = 5): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  if (ms > 0) await new Promise((res) => setTimeout(res, ms));
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

beforeEach(() => {
  installFetchStub();
  __roomRegistryInternals.reset();
});

afterEach(() => {
  __roomRegistryInternals.reset();
  restoreFetch();
});

// --- tests -------------------------------------------------------------

describe("useRoom WS push path", () => {
  test("WS connected + engine wired → subscribeRoom called, no polling interval", async () => {
    const engine = makeFakeEngine();
    const room = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      25, // would tick every 25ms IF polling were active
      engine as any,
    );
    expect(engine._subscribeCalls).toEqual([ROOM]);
    expect(__roomRegistryInternals.isWsAttached(room)).toBe(true);
    expect(__roomRegistryInternals.isPolling(room)).toBe(false);

    // Wait through what WOULD have been multiple polling ticks. No
    // GET /api/rooms/<room> should ever land.
    await flush(80);
    expect(recorded.filter(isHeartbeat).length).toBe(0);

    // Engine pushes a snapshot — the hook's local cache should reflect
    // it once the registry's pulse runs.
    engine.pushSnapshot(ROOM, [
      { user_id: "alice", joined_at: "t1" },
      { user_id: USER, joined_at: "t2" },
    ]);
    await flush();
    // The registry filters out the current user → only alice should
    // remain.
    expect(room.peers.map((p) => p.user_id)).toEqual(["alice"]);
  });

  test("inbound room-update action:join updates peers", async () => {
    const engine = makeFakeEngine();
    const room = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      1_000,
      engine as any,
    );
    // Let the HTTP join settle first so its (empty) snapshot doesn't
    // race the WS-pushed members. In production the WS snapshot
    // typically lands shortly AFTER the join HTTP response — this
    // ordering matches that.
    await flush();
    engine.pushSnapshot(ROOM, []);
    engine.pushUpdate(ROOM, "join", { user_id: "bob", joined_at: "t3" });
    await flush();
    expect(room.peers.map((p) => p.user_id)).toEqual(["bob"]);
  });

  test("server pushes NOT_IN_ROOM error → surfaces via room.error, isConnected false, no retry", async () => {
    const engine = makeFakeEngine();
    const room = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      1_000,
      engine as any,
    );
    // Let the HTTP join settle first so its success doesn't clobber
    // the WS-pushed error state.
    await flush();
    engine.pushError(ROOM, "NOT_IN_ROOM", "not a member");
    await flush();
    expect(typeof room.error).toBe("string");
    expect(room.error ?? "").toContain("not a member");
    expect(room.isConnected).toBe(false);
    // Snapshot-timeout fallback should NOT have engaged (error path
    // cancels the backwards-compat timer).
    await flush(80);
    expect(__roomRegistryInternals.isPolling(room)).toBe(false);
  });

  test("WS down → polling fallback fires; reconnect promotes back to push", async () => {
    const engine = makeFakeEngine();
    engine.setWsConnected(false);
    const room = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      25,
      engine as any,
    );
    // No WS — no subscribeRoom call, polling active.
    expect(engine._subscribeCalls).toEqual([]);
    expect(__roomRegistryInternals.isPolling(room)).toBe(true);

    await flush(80);
    expect(recorded.filter(isHeartbeat).length).toBeGreaterThan(0);

    // Flip to connected → registry promotes to push, polling stops.
    engine.setWsConnected(true);
    await flush();
    expect(__roomRegistryInternals.isWsAttached(room)).toBe(true);
    expect(__roomRegistryInternals.isPolling(room)).toBe(false);
    expect(engine._subscribeCalls).toEqual([ROOM]);
  });

  test("backwards-compat: WS connected but no snapshot in 2s → fall back to polling", async () => {
    const engine = makeFakeEngine();
    // Crucial: engine.subscribeRoom returns successfully but never
    // pushes a snapshot. That mirrors talking to a pre-v0.3.214 server
    // which silently drops the room-subscribe frame.
    const room = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      25,
      engine as any,
    );
    expect(__roomRegistryInternals.isWsAttached(room)).toBe(true);
    expect(__roomRegistryInternals.isPolling(room)).toBe(false);

    // The push-snapshot timeout is 2s in the implementation. Wait
    // long enough for it to fire AND let one polling tick happen.
    await flush(2_100);
    expect(__roomRegistryInternals.isWsAttached(room)).toBe(false);
    expect(__roomRegistryInternals.isPolling(room)).toBe(true);
    expect(recorded.filter(isHeartbeat).length).toBeGreaterThan(0);
  });

  test("StrictMode double-mount on WS path: still one engine.subscribeRoom call", async () => {
    const engine = makeFakeEngine();
    const r1 = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      1_000,
      engine as any,
    );
    const r2 = __roomRegistryInternals.acquire(
      BASE,
      ROOM,
      USER,
      TOKEN,
      {},
      1_000,
      engine as any,
    );
    expect(r1).toBe(r2);
    // Registry dedup → only one acquire path → only one subscribe.
    expect(engine._subscribeCalls).toEqual([ROOM]);
    __roomRegistryInternals.release(r1);
    __roomRegistryInternals.release(r2);
    await flush(20);
    // One subscribe, one unsubscribe — no fanout.
    expect(engine._unsubscribeCalls).toEqual([ROOM]);
  });
});
