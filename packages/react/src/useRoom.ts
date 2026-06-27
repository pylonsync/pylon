"use client";

import { useState, useEffect, useCallback, useRef } from 'react';
import { pylonFetch, type RoomMember, type SyncEngine } from '@pylonsync/sync';
import { getBaseUrl, getReactStorage, storageKey } from './index';
import { getSync } from './db';

/** Project an engine RoomMember (data optional) into a RoomPeer (data
 *  required). The wire frame always carries a `data` slot — empty objects
 *  are valid — so the normalization is purely a type contract. */
function memberToPeer(m: RoomMember): RoomPeer {
  return {
    user_id: m.user_id,
    joined_at: m.joined_at,
    data: m.data ?? {},
  };
}

// ---------------------------------------------------------------------------
// Room types
// ---------------------------------------------------------------------------

export interface RoomPeer {
  user_id: string;
  data: any;
  joined_at: string;
}

export interface RoomSnapshot {
  room: string;
  peers: RoomPeer[];
}

export interface UseRoomOptions {
  /** Base URL of the pylon server. */
  baseUrl?: string;
  /** Auth token for API requests. */
  token?: string;
  /** Initial presence data sent on join. */
  initialPresence?: Record<string, any>;
  /**
   * How often (ms) to poll for peer updates WHEN the WebSocket
   * push channel is unavailable. With a connected WebSocket the SDK
   * uses `room-subscribe` push and the polling timer never fires.
   * Defaults to 5_000.
   */
  heartbeatInterval?: number;
}

export interface UseRoomReturn {
  /** Current peers in the room (excluding self). */
  peers: RoomPeer[];
  /** Whether currently connected to the room. */
  isConnected: boolean;
  /** Update your presence data (e.g. cursor position, typing status). */
  setPresence: (data: Record<string, any>) => void;
  /** Broadcast a message to the room on a given topic. */
  broadcast: (topic: string, data: any) => void;
  /** Leave the room manually. */
  leave: () => void;
  /** Error message, if any. */
  error: string | null;
}

// ---------------------------------------------------------------------------
// Shared room registry — dedupes joins/heartbeats across components
// ---------------------------------------------------------------------------
//
// Apps routinely call `useRoom(roomId, userId)` from 3+ components for the
// same channel (presence count, presence avatars, typing indicator,
// composer, …). Without deduping that explodes into N joins on mount, N
// leaves on unmount, and N×interval heartbeat polls every tick. This
// registry collapses all calls with the same identity tuple into a single
// network footprint: one join, one interval (polling-fallback only),
// one leave.
//
// Identity key includes everything that would change the network shape:
// baseUrl + token (different server / auth) + roomId + userId.
//
// StrictMode safety: when refcount drops to 0, the leave + interval
// teardown is scheduled on a microtask. If the same hook re-mounts
// synchronously in the same tick (StrictMode double-mount, fast tab
// switch, parent re-key), the pending teardown is cancelled and the room
// stays alive. The `joined` race-fix from the in-effect implementation
// is preserved: leave only fires once the original join has actually
// landed on the server.
//
// WS-push integration (v0.3.216): when the sync engine's transport is
// websocket AND currently connected, the registry attaches to
// `engine.subscribeRoom(roomId, cb)` and stops the polling interval.
// The engine's room registry handles the wire-level refcount
// independently. When the WS isn't available (SSE transport, polling
// transport, OR WS not yet connected), the registry runs the legacy 5s
// polling loop. We re-evaluate on connection-status changes so a tab
// that mounts while offline polls until the WS comes up, then quietly
// switches to push.

/** Window (ms) we wait for a `room-snapshot` after sending the first
 *  `room-subscribe`. If nothing lands by then we assume the server is
 *  pre-v0.3.214 and doesn't speak the push protocol — fall back to
 *  polling. Sized generously enough to absorb a slow link + the
 *  server's RoomManager lookup; short enough that a misconfigured
 *  setup recovers without the user noticing. */
const PUSH_SNAPSHOT_TIMEOUT_MS = 2_000;

interface SharedRoom {
  /** Number of live React hooks holding this room. */
  refs: number;
  /** Has the join() request resolved with a server response? */
  joined: boolean;
  /** Cached snapshot for late subscribers (mounting after the join landed). */
  peers: RoomPeer[];
  isConnected: boolean;
  error: string | null;
  /** Last presence data the caller pushed via setPresence. */
  presence: Record<string, any>;
  /** Active heartbeat poll (only set when polling fallback is active). */
  interval: ReturnType<typeof setInterval> | null;
  /** Pending teardown — set when refcount hit 0 but we haven't actually
   *  fired leave yet, to absorb StrictMode mount/unmount/mount races. */
  pendingTeardown: ReturnType<typeof setTimeout> | null;
  /** Subscriber callbacks; each useRoom hook registers one. */
  subs: Set<() => void>;
  /** Transport identity (frozen at create time — changing it changes the key). */
  baseUrl: string;
  token: string | undefined;
  roomId: string;
  userId: string;
  heartbeatInterval: number;
  /** Sync engine instance the room is attached to (for WS push).
   *  `null` when none provided / running in SSR or test harness that
   *  never invoked init(). */
  engine: SyncEngine | null;
  /** Unsubscribe handle returned by engine.subscribeRoom. `null` when
   *  we're polling. Mutually exclusive with `interval` — exactly one
   *  is set whenever the room is live. */
  wsUnsubscribe: (() => void) | null;
  /** Timer that fires PUSH_SNAPSHOT_TIMEOUT_MS after we send the first
   *  room-subscribe. If a snapshot lands first we cancel it; if it
   *  fires we assume the server doesn't speak the push protocol and
   *  fall back to polling. */
  pushSnapshotTimer: ReturnType<typeof setTimeout> | null;
  /** Connection-status unsubscribe — fires whenever the engine's
   *  transport state changes so we can flip between push and polling. */
  connectionStatusUnsubscribe: (() => void) | null;
}

const rooms = new Map<string, SharedRoom>();

function roomKey(
  baseUrl: string,
  roomId: string,
  userId: string,
  token: string | undefined,
): string {
  return `${baseUrl}|${roomId}|${userId}|${token ?? ''}`;
}

function notify(room: SharedRoom): void {
  for (const cb of room.subs) cb();
}

function transportFor(room: SharedRoom) {
  return { baseUrl: room.baseUrl, token: room.token };
}

function roomKeyFor(room: SharedRoom): string {
  return roomKey(room.baseUrl, room.roomId, room.userId, room.token);
}

/** Begin the 5s GET /api/rooms/<room> polling loop. Idempotent — if
 *  a polling timer is already running, no-op. */
function startHeartbeat(room: SharedRoom): void {
  if (room.interval) return;
  room.interval = setInterval(async () => {
    // Guard against late-firing intervals after teardown — if the room
    // was already evicted from the registry, drop this tick.
    if (!rooms.has(roomKeyFor(room))) return;
    try {
      const body = await pylonFetch<{ members?: RoomPeer[] }>(
        transportFor(room),
        `/api/rooms/${encodeURIComponent(room.roomId)}`,
      );
      if (!rooms.has(roomKeyFor(room))) return;
      const next = (body.members ?? []).filter((p) => p.user_id !== room.userId);
      room.peers = next;
      notify(room);
    } catch {
      // Swallow — next heartbeat will retry.
    }
  }, room.heartbeatInterval);
}

/** Stop the polling timer if running. */
function stopHeartbeat(room: SharedRoom): void {
  if (room.interval) {
    clearInterval(room.interval);
    room.interval = null;
  }
}

/** Attach to the engine's WS room subscription. Pulls in the current
 *  snapshot (or, when one lands, replaces the cached peers). If the
 *  initial subscribe doesn't surface a snapshot within
 *  PUSH_SNAPSHOT_TIMEOUT_MS, fall back to polling — that's the
 *  backwards-compat path for pre-v0.3.214 servers. */
function startWsPush(room: SharedRoom): void {
  const engine = room.engine;
  if (!engine) return;
  if (room.wsUnsubscribe) return; // already attached
  stopHeartbeat(room);
  room.wsUnsubscribe = engine.subscribeRoom(room.roomId, () => {
    if (!rooms.has(roomKeyFor(room))) return;
    const members = engine.getRoomMembers(room.roomId);
    const err = engine.getRoomError(room.roomId);
    if (err) {
      // Snapshot landing implies the server speaks the push protocol.
      // Cancel the backwards-compat timer; we're staying on push.
      if (room.pushSnapshotTimer) {
        clearTimeout(room.pushSnapshotTimer);
        room.pushSnapshotTimer = null;
      }
      room.error =
        err.code === 'NOT_IN_ROOM'
          ? 'You are not a member of this room.'
          : (err.message ?? err.code);
      room.isConnected = false;
      notify(room);
      return;
    }
    if (members !== null) {
      if (room.pushSnapshotTimer) {
        clearTimeout(room.pushSnapshotTimer);
        room.pushSnapshotTimer = null;
      }
      room.peers = members
        .filter((m) => m.user_id !== room.userId)
        .map(memberToPeer);
      room.isConnected = true;
      room.error = null;
      notify(room);
    }
  });
  // Cached state inside the engine's registry may already be populated
  // (we are a late subscriber to an existing engine-level sub). Pull
  // it once so the hook re-renders immediately without waiting for the
  // next push.
  const cachedMembers = engine.getRoomMembers(room.roomId);
  if (cachedMembers !== null) {
    room.peers = cachedMembers
      .filter((m) => m.user_id !== room.userId)
      .map(memberToPeer);
    room.isConnected = true;
    notify(room);
  } else if (!room.pushSnapshotTimer) {
    // Arm the backwards-compat timer. If no snapshot lands in 2s, the
    // server is presumed pre-v0.3.214 and we revert to polling. The
    // timer is cancelled the moment a real snapshot OR a real error
    // lands (both paths above clear it).
    room.pushSnapshotTimer = setTimeout(() => {
      room.pushSnapshotTimer = null;
      if (!rooms.has(roomKeyFor(room))) return;
      // No snapshot — tear down the WS sub and fall back to polling.
      // The engine's room-unsubscribe ships harmlessly even on old
      // servers (unknown frames are ignored).
      if (room.wsUnsubscribe) {
        room.wsUnsubscribe();
        room.wsUnsubscribe = null;
      }
      startHeartbeat(room);
    }, PUSH_SNAPSHOT_TIMEOUT_MS);
  }
}

/** Detach the engine WS sub if attached. */
function stopWsPush(room: SharedRoom): void {
  if (room.wsUnsubscribe) {
    room.wsUnsubscribe();
    room.wsUnsubscribe = null;
  }
  if (room.pushSnapshotTimer) {
    clearTimeout(room.pushSnapshotTimer);
    room.pushSnapshotTimer = null;
  }
}

/** Pick the right delivery channel for the room based on the engine's
 *  current transport state. Called on initial acquire AND whenever the
 *  engine's connection status changes — a room that started on polling
 *  promotes to push the moment the WS opens, and vice versa. */
function evaluateDelivery(room: SharedRoom): void {
  const engine = room.engine;
  if (!engine) {
    // No engine wired in — pure HTTP polling mode. Idempotent.
    stopWsPush(room);
    startHeartbeat(room);
    return;
  }
  if (engine.isWebSocketConnected()) {
    startWsPush(room);
  } else {
    stopWsPush(room);
    startHeartbeat(room);
  }
}

function joinRoom(room: SharedRoom): void {
  // Fire-and-forget — the join promise resolves into the shared state
  // and notifies all subscribers. Late subscribers either pick up the
  // already-resolved snapshot from `room.peers` or the next push /
  // heartbeat.
  pylonFetch<{ snapshot?: { peers?: RoomPeer[] } }>(
    transportFor(room),
    '/api/rooms/join',
    {
      method: 'POST',
      json: { room: room.roomId, user_id: room.userId, data: room.presence },
    },
  )
    .then((body) => {
      // The room may have been torn down before join landed (mount →
      // unmount → no remount). In that case the entry is gone from the
      // registry; bail and let the leave path (if it ran) do its thing.
      if (!rooms.has(roomKeyFor(room))) return;
      room.joined = true;
      room.isConnected = true;
      room.error = null;
      if (body.snapshot?.peers) {
        room.peers = body.snapshot.peers.filter((p) => p.user_id !== room.userId);
      }
      notify(room);
    })
    .catch((e: any) => {
      if (!rooms.has(roomKeyFor(room))) return;
      room.error = e?.message ?? 'Failed to join room';
      notify(room);
    });
}

function leaveRoom(room: SharedRoom): void {
  // Only ship leave if join actually landed — matches the original
  // StrictMode race-fix. Server leave is idempotent now, but skipping
  // the call still saves a round trip on every double-mount in dev.
  if (room.joined) {
    pylonFetch(transportFor(room), '/api/rooms/leave', {
      method: 'POST',
      json: { room: room.roomId, user_id: room.userId },
    }).catch(() => {});
  }
}

function acquireRoom(
  baseUrl: string,
  roomId: string,
  userId: string,
  token: string | undefined,
  initialPresence: Record<string, any>,
  heartbeatInterval: number,
  engine: SyncEngine | null,
): SharedRoom {
  const key = roomKey(baseUrl, roomId, userId, token);
  let room = rooms.get(key);
  if (room) {
    // Absorb a pending teardown — caller is about to incref past 0
    // again before the deferred leave fired. Cancel it so we don't
    // tear down a still-live room.
    if (room.pendingTeardown) {
      clearTimeout(room.pendingTeardown);
      room.pendingTeardown = null;
    }
    room.refs += 1;
    return room;
  }
  room = {
    refs: 1,
    joined: false,
    peers: [],
    isConnected: false,
    error: null,
    presence: initialPresence,
    interval: null,
    pendingTeardown: null,
    subs: new Set(),
    baseUrl,
    token,
    roomId,
    userId,
    heartbeatInterval,
    engine,
    wsUnsubscribe: null,
    pushSnapshotTimer: null,
    connectionStatusUnsubscribe: null,
  };
  rooms.set(key, room);
  joinRoom(room);
  // Watch connection-status so the room flips from polling to push (or
  // back) as the WS opens / drops. Only useful when an engine is wired
  // in; without one, the hook is HTTP-only and the status never matters.
  if (engine) {
    room.connectionStatusUnsubscribe = subscribeConnectionStatus(engine, () => {
      const r = rooms.get(roomKeyFor(room!));
      if (r) evaluateDelivery(r);
    });
  }
  evaluateDelivery(room);
  return room;
}

function releaseRoom(room: SharedRoom): void {
  room.refs -= 1;
  if (room.refs > 0) return;
  // Defer the actual teardown to the next macrotask. React StrictMode
  // unmounts and re-mounts every effect synchronously in dev; without
  // this defer we'd ship join → leave → join on every render. A 0ms
  // setTimeout is enough — the synchronous remount lands first and
  // cancels the pending teardown via `acquireRoom`.
  if (room.pendingTeardown) clearTimeout(room.pendingTeardown);
  room.pendingTeardown = setTimeout(() => {
    // Re-check refcount — a remount might have incref'd inside the
    // same tick after the timer was scheduled.
    if (room.refs > 0) {
      room.pendingTeardown = null;
      return;
    }
    stopHeartbeat(room);
    stopWsPush(room);
    if (room.connectionStatusUnsubscribe) {
      room.connectionStatusUnsubscribe();
      room.connectionStatusUnsubscribe = null;
    }
    rooms.delete(roomKeyFor(room));
    room.pendingTeardown = null;
    leaveRoom(room);
  }, 0);
}

/**
 * Best-effort subscription to the engine's connection-status stream.
 * The sync engine notifies via `store.notify()` on every status change;
 * we listen on the LocalStore's subscribe surface and poll the status
 * inside the callback. Returns an unsubscribe.
 *
 * This module deliberately doesn't import `useSyncStatus` (a React
 * hook) — the registry runs outside React, and the only thing we need
 * is a notify-on-change pulse. The store's subscribe method gives us
 * exactly that without pulling in another dep.
 */
function subscribeConnectionStatus(
  engine: SyncEngine,
  cb: () => void,
): () => void {
  // The engine's LocalStore notify() fires on every status flip
  // (setConnectionStatus calls store.notify). Subscribing to that is
  // a cheap way to observe transport transitions.
  const store = (engine as unknown as { store?: { subscribe: (fn: () => void) => () => void } }).store;
  if (!store?.subscribe) {
    return () => {};
  }
  let lastStatus = engine.connectionStatus();
  return store.subscribe(() => {
    const nextStatus = engine.connectionStatus();
    if (nextStatus === lastStatus) return;
    lastStatus = nextStatus;
    cb();
  });
}

// Exported for tests — lets the regression test reset registry state
// between cases so leaked rooms from one test can't bleed into the next,
// and drive the refcount through the same code path the hook uses.
export const __roomRegistryInternals = {
  reset(): void {
    for (const room of rooms.values()) {
      if (room.pendingTeardown) clearTimeout(room.pendingTeardown);
      stopHeartbeat(room);
      stopWsPush(room);
      if (room.connectionStatusUnsubscribe) {
        room.connectionStatusUnsubscribe();
        room.connectionStatusUnsubscribe = null;
      }
    }
    rooms.clear();
  },
  acquire(
    baseUrl: string,
    roomId: string,
    userId: string,
    token: string | undefined,
    initialPresence: Record<string, any>,
    heartbeatInterval: number,
    engine: SyncEngine | null = null,
  ): SharedRoom {
    return acquireRoom(
      baseUrl,
      roomId,
      userId,
      token,
      initialPresence,
      heartbeatInterval,
      engine,
    );
  },
  release: releaseRoom,
  size(): number {
    return rooms.size;
  },
  /** Diagnostic: is this room currently on the WS push path? */
  isWsAttached(room: SharedRoom): boolean {
    return room.wsUnsubscribe !== null;
  },
  /** Diagnostic: is the polling timer running for this room? */
  isPolling(room: SharedRoom): boolean {
    return room.interval !== null;
  },
};

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Subscribe to a real-time room. Joins on mount, leaves on unmount, and
 * receives peer updates over the sync engine's WebSocket push channel
 * (server v0.3.214+). When the WS isn't available (SSE transport, polling
 * transport, OR WS not yet connected) the hook automatically falls back
 * to a 5s `GET /api/rooms/<room>` polling loop — same behaviour as
 * v0.3.213 and earlier.
 *
 * Multiple components calling `useRoom` with the same `(roomId, userId,
 * baseUrl, token)` tuple share a single underlying join + subscription —
 * mounting it from 5 components costs one POST /api/rooms/join, one
 * room-subscribe on the wire, and one POST /api/rooms/leave.
 *
 * ```tsx
 * const { peers, isConnected, setPresence, broadcast, leave, error } = useRoom(
 *   "project-42",
 *   currentUser.id,
 *   { baseUrl: "http://localhost:4321", token }
 * );
 * ```
 */
/**
 * Read the current pylon token from the configured storage adapter
 * (default: localStorage on web, AsyncStorage on RN, etc). Keeps the
 * hook working even when the caller doesn't explicitly thread a token
 * — otherwise every useRoom request hits the server as anonymous and
 * 401s under any authenticated room policy.
 */
function readStoredToken(): string | undefined {
  return getReactStorage().get(storageKey('token')) ?? undefined;
}

/** Best-effort engine accessor. SSR / test-harness use-cases that
 *  never invoked `init()` end up here with a null engine and the
 *  hook silently runs in HTTP-only polling mode. */
function tryGetSync(): SyncEngine | null {
  try {
    return getSync();
  } catch {
    return null;
  }
}

export function useRoom(
  roomId: string,
  userId: string,
  options: UseRoomOptions = {},
): UseRoomReturn {
  const {
    // Fall back to the globally configured baseUrl so room requests don't
    // land on the Vite dev origin (localhost:5173) and 404 when the caller
    // forgets to pass one.
    baseUrl = getBaseUrl(),
    token: explicitToken,
    initialPresence = {},
    heartbeatInterval = 5_000,
  } = options;
  // Resolve at render time rather than hook-creation time so the room
  // reconnects with a fresh token after login.
  const token = explicitToken ?? readStoredToken();

  const [peers, setPeers] = useState<RoomPeer[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Remember the most recent shared room this hook is attached to so
  // setPresence / broadcast / leave can route through it without
  // re-running the lifecycle effect on every render.
  const roomRef = useRef<SharedRoom | null>(null);

  useEffect(() => {
    const engine = tryGetSync();
    const room = acquireRoom(
      baseUrl,
      roomId,
      userId,
      token,
      initialPresence,
      heartbeatInterval,
      engine,
    );
    roomRef.current = room;

    // Pull the current shared snapshot immediately so a late subscriber
    // doesn't have to wait one heartbeat to see who's in the room.
    setPeers(room.peers);
    setIsConnected(room.isConnected);
    setError(room.error);

    const sub = () => {
      setPeers(room.peers);
      setIsConnected(room.isConnected);
      setError(room.error);
    };
    room.subs.add(sub);

    return () => {
      room.subs.delete(sub);
      if (roomRef.current === room) roomRef.current = null;
      releaseRoom(room);
    };
    // initialPresence is intentionally excluded — it's a "set once on
    // first mount" value, and treating an inline-literal default as a
    // dep would thrash the lifecycle on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [roomId, userId, baseUrl, token, heartbeatInterval]);

  // ------- actions -------
  //
  // These read from `roomRef.current` so they always hit the live shared
  // room without needing the effect to re-run when callers change.

  const setPresence = useCallback(
    (data: Record<string, any>) => {
      const room = roomRef.current;
      if (room) room.presence = data;
      pylonFetch(
        { baseUrl, token },
        '/api/rooms/presence',
        {
          method: 'POST',
          json: { room: roomId, user_id: userId, data },
        },
      ).catch(() => {});
    },
    [roomId, userId, baseUrl, token],
  );

  const broadcast = useCallback(
    (topic: string, data: any) => {
      pylonFetch(
        { baseUrl, token },
        '/api/rooms/broadcast',
        {
          method: 'POST',
          json: { room: roomId, user_id: userId, topic, data },
        },
      ).catch(() => {});
    },
    [roomId, userId, baseUrl, token],
  );

  const leave = useCallback(() => {
    // Manual leave — bypasses the shared lifecycle and tells the server
    // we're gone immediately. The next effect cleanup will still try to
    // release the refcount; the join-guard means a duplicate leave is
    // skipped if needed, and the server is idempotent anyway.
    pylonFetch(
      { baseUrl, token },
      '/api/rooms/leave',
      {
        method: 'POST',
        json: { room: roomId, user_id: userId },
      },
    ).catch(() => {});
    setIsConnected(false);
    setPeers([]);
  }, [roomId, userId, baseUrl, token]);

  return { peers, isConnected, setPresence, broadcast, leave, error };
}
