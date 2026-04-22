import { useState, useEffect, useCallback, useRef } from 'react';
import { getBaseUrl } from './index';

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
  /** Base URL of the statecraft server. */
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
// Hook
// ---------------------------------------------------------------------------

/**
 * Subscribe to a real-time room. Joins on mount, leaves on unmount, and
 * polls for peer updates on a configurable interval.
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
 * Read the current statecraft token from localStorage, matching the
 * convention used by `callFn` and the sync engine. Keeps the hook working
 * even when the caller doesn't explicitly thread a token — otherwise every
 * useRoom request hits the server as anonymous and 401s under any
 * authenticated room policy.
 */
function readStoredToken(): string | undefined {
  if (typeof window === 'undefined' || !window.localStorage) return undefined;
  return window.localStorage.getItem('statecraft_token') ?? undefined;
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

  const presenceRef = useRef(initialPresence);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Stable header builder -- only changes when `token` changes.
  const headers = useCallback((): Record<string, string> => {
    const h: Record<string, string> = { 'Content-Type': 'application/json' };
    if (token) h['Authorization'] = `Bearer ${token}`;
    return h;
  }, [token]);

  // ------- lifecycle: join / heartbeat / leave -------
  useEffect(() => {
    let mounted = true;

    const join = async () => {
      try {
        const res = await fetch(`${baseUrl}/api/rooms/join`, {
          method: 'POST',
          headers: headers(),
          body: JSON.stringify({
            room: roomId,
            user_id: userId,
            data: presenceRef.current,
          }),
        });
        const body = await res.json();
        if (!mounted) return;

        if (res.ok) {
          setIsConnected(true);
          setError(null);
          if (body.snapshot?.peers) {
            setPeers(
              (body.snapshot.peers as RoomPeer[]).filter(
                (p) => p.user_id !== userId,
              ),
            );
          }
        } else {
          setError(body.error?.message || 'Failed to join room');
        }
      } catch (e: any) {
        if (mounted) setError(e.message);
      }
    };

    join();

    // Poll for peer list updates.
    intervalRef.current = setInterval(async () => {
      if (!mounted) return;
      try {
        const res = await fetch(
          `${baseUrl}/api/rooms/${encodeURIComponent(roomId)}`,
          { headers: headers() },
        );
        if (res.ok) {
          const body = await res.json();
          if (mounted) {
            setPeers(
              ((body.members ?? []) as RoomPeer[]).filter(
                (p) => p.user_id !== userId,
              ),
            );
          }
        }
      } catch {
        // Swallow -- next heartbeat will retry.
      }
    }, heartbeatInterval);

    return () => {
      mounted = false;
      if (intervalRef.current) clearInterval(intervalRef.current);

      // Best-effort leave on unmount (fire-and-forget).
      fetch(`${baseUrl}/api/rooms/leave`, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ room: roomId, user_id: userId }),
      }).catch(() => {});
    };
    // Re-run the entire lifecycle when identity or connection details change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [roomId, userId, baseUrl, token, heartbeatInterval]);

  // ------- actions -------

  const setPresence = useCallback(
    (data: Record<string, any>) => {
      presenceRef.current = data;
      fetch(`${baseUrl}/api/rooms/presence`, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ room: roomId, user_id: userId, data }),
      }).catch(() => {});
    },
    [roomId, userId, baseUrl, headers],
  );

  const broadcast = useCallback(
    (topic: string, data: any) => {
      fetch(`${baseUrl}/api/rooms/broadcast`, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ room: roomId, user_id: userId, topic, data }),
      }).catch(() => {});
    },
    [roomId, userId, baseUrl, headers],
  );

  const leave = useCallback(() => {
    fetch(`${baseUrl}/api/rooms/leave`, {
      method: 'POST',
      headers: headers(),
      body: JSON.stringify({ room: roomId, user_id: userId }),
    }).catch(() => {});
    setIsConnected(false);
    setPeers([]);
  }, [roomId, userId, baseUrl, headers]);

  return { peers, isConnected, setPresence, broadcast, leave, error };
}
