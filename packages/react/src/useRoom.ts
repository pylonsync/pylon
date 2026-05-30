"use client";

import { useState, useEffect, useCallback, useRef } from 'react';
import { pylonFetch } from '@pylonsync/sync';
import { getBaseUrl, getReactStorage, storageKey } from './index';

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
  /** How often to poll for peer updates (ms). Defaults to 5 000. */
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
// network footprint: one join, one interval, one leave.
//
// Identity key includes everything that would change the network shape:
// baseUrl + token (different server / auth) + roomId + userId.
//
// StrictMode safety: when refcount drops to 0, the leave + interval
// teardown is scheduled on a microtask. If the same hook re-mounts
// synchronously in the same tick (StrictMode double-mount, fast tab
// switch, parent re-key), the pending teardown is cancelled and the room
// stays alive. The existing `joined` race-fix from the in-effect
// implementation is preserved here too: leave only fires once the
// original join has actually landed on the server.

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
  /** Active heartbeat poll. */
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
      // Swallow — next heartbeat will retry. Matches prior behaviour.
    }
  }, room.heartbeatInterval);
}

function roomKeyFor(room: SharedRoom): string {
  return roomKey(room.baseUrl, room.roomId, room.userId, room.token);
}

function joinRoom(room: SharedRoom): void {
  // Fire-and-forget — the join promise resolves into the shared state
  // and notifies all subscribers. Late subscribers either pick up the
  // already-resolved snapshot from `room.peers` or the next heartbeat.
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
  };
  rooms.set(key, room);
  joinRoom(room);
  startHeartbeat(room);
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
    if (room.interval) {
      clearInterval(room.interval);
      room.interval = null;
    }
    rooms.delete(roomKeyFor(room));
    room.pendingTeardown = null;
    leaveRoom(room);
  }, 0);
}

// Exported for tests — lets the regression test reset registry state
// between cases so leaked rooms from one test can't bleed into the next,
// and drive the refcount through the same code path the hook uses.
export const __roomRegistryInternals = {
  reset(): void {
    for (const room of rooms.values()) {
      if (room.pendingTeardown) clearTimeout(room.pendingTeardown);
      if (room.interval) clearInterval(room.interval);
    }
    rooms.clear();
  },
  acquire: acquireRoom,
  release: releaseRoom,
  size(): number {
    return rooms.size;
  },
};

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Subscribe to a real-time room. Joins on mount, leaves on unmount, and
 * polls for peer updates on a configurable interval.
 *
 * Multiple components calling `useRoom` with the same `(roomId, userId,
 * baseUrl, token)` tuple share a single underlying join + heartbeat —
 * mounting it from 5 components costs one POST /api/rooms/join, one
 * heartbeat poll, and one POST /api/rooms/leave.
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
    const room = acquireRoom(
      baseUrl,
      roomId,
      userId,
      token,
      initialPresence,
      heartbeatInterval,
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
