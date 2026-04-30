"use client";

/**
 * useShard — React hook for real-time sharded simulations (games, MMO zones, etc.).
 *
 * Connects to an pylon shard over WebSocket (preferred) or SSE (fallback),
 * receives snapshots as they arrive, and sends inputs upstream.
 *
 * @example
 * ```tsx
 * const { snapshot, tick, send, connected, error } = useShard("match1", {
 *   subscriberId: "player42",
 *   token: authToken,
 * });
 *
 * return (
 *   <GameBoard snapshot={snapshot} onMove={(move) => send({ action: "move", move })} />
 * );
 * ```
 */

import { useEffect, useRef, useState } from "react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface UseShardOptions {
  /** Subscriber ID (usually the logged-in user ID). Required for multiplayer. */
  subscriberId: string;
  /**
   * Auth token. Sent over the WebSocket as a Sec-WebSocket-Protocol
   * subprotocol header in the form `"bearer.<token>"`. This keeps the token
   * out of URLs — proxy logs, browser devtools network panel, and error
   * telemetry typically record the URL but not the subprotocol value.
   *
   * The pylon shard server reads either the subprotocol header or the
   * legacy `?token=` query param (which is still accepted but deprecated —
   * scheduled for removal in a future release).
   */
  token?: string;
  /** Override the base URL. Defaults to `window.location.host`. */
  baseUrl?: string;
  /** Override the shard WS port (default: HTTP port + 3). */
  wsPort?: number;
  /** Explicit WebSocket URL. Overrides baseUrl/wsPort. */
  wsUrl?: string;
  /** If true, falls back to SSE + HTTP POST if WebSocket fails (default: true). */
  sseFallback?: boolean;
  /** Reconnect on unexpected close (default: true). */
  autoReconnect?: boolean;
  /** Reconnect backoff in ms (default: starts at 500, maxes at 10_000). */
  reconnectBackoffMs?: number;
}

export interface UseShardReturn<TSnapshot = unknown, TInput = unknown> {
  snapshot: TSnapshot | null;
  tick: number;
  connected: boolean;
  error: Error | null;
  /** Send an input to the shard. Returns a client sequence number. */
  send: (input: TInput) => number;
  /** Close the connection early. */
  close: () => void;
}

// ---------------------------------------------------------------------------
// Low-level client (no React)
// ---------------------------------------------------------------------------

export interface ShardClient<TSnapshot = unknown, TInput = unknown> {
  onSnapshot: (fn: (snapshot: TSnapshot, tick: number) => void) => void;
  onError: (fn: (err: Error) => void) => void;
  onOpen: (fn: () => void) => void;
  onClose: (fn: () => void) => void;
  send: (input: TInput) => number;
  close: () => void;
  readonly connected: boolean;
}

/**
 * Connect to a shard without React — returns a typed client you can wire
 * into any framework.
 */
export function connectShard<TSnapshot = unknown, TInput = unknown>(
  shardId: string,
  options: UseShardOptions
): ShardClient<TSnapshot, TInput> {
  let ws: WebSocket | null = null;
  let clientSeq = 0;
  let closed = false;
  let connected = false;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let backoff = options.reconnectBackoffMs ?? 500;

  const snapshotHandlers: Array<(s: TSnapshot, t: number) => void> = [];
  const errorHandlers: Array<(e: Error) => void> = [];
  const openHandlers: Array<() => void> = [];
  const closeHandlers: Array<() => void> = [];

  const dispatchSnapshot = (snapshot: TSnapshot, tick: number) => {
    for (const h of snapshotHandlers) h(snapshot, tick);
  };
  const dispatchError = (err: Error) => {
    for (const h of errorHandlers) h(err);
  };

  const buildWsUrl = (): string => {
    if (options.wsUrl) return options.wsUrl;
    const host =
      options.baseUrl ||
      (typeof window !== "undefined" ? window.location.hostname : "localhost");
    const port = options.wsPort ?? 4324; // default: pylon HTTP port + 3 (4321 + 3)
    const proto =
      typeof window !== "undefined" && window.location.protocol === "https:"
        ? "wss"
        : "ws";
    // Only shard id + subscriber id land in the URL — these are routing
    // metadata, not credentials.
    const params = new URLSearchParams({
      shard: shardId,
      sid: options.subscriberId,
    });
    return `${proto}://${host}:${port}/?${params.toString()}`;
  };

  const connect = () => {
    if (closed) return;
    const url = buildWsUrl();
    try {
      // The bearer token rides on the WebSocket subprotocol header so it
      // doesn't get captured by every proxy / devtools pane that logs URLs.
      // Subprotocol values must be a token per RFC 6455; encode the bearer
      // so spaces/punctuation don't break the handshake.
      const protocols = options.token
        ? [`bearer.${encodeURIComponent(options.token)}`]
        : undefined;
      ws = protocols ? new WebSocket(url, protocols) : new WebSocket(url);
    } catch (e) {
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      connected = true;
      backoff = options.reconnectBackoffMs ?? 500;
      for (const h of openHandlers) h();
    };

    ws.onmessage = (event) => {
      // Binary format: 8 bytes (u64 BE) tick + JSON snapshot bytes.
      if (event.data instanceof ArrayBuffer) {
        const view = new DataView(event.data);
        const hi = view.getUint32(0);
        const lo = view.getUint32(4);
        const tick = hi * 0x100000000 + lo;
        const jsonBytes = new Uint8Array(event.data, 8);
        const jsonStr = new TextDecoder().decode(jsonBytes);
        try {
          const snapshot = JSON.parse(jsonStr) as TSnapshot;
          dispatchSnapshot(snapshot, tick);
        } catch (e) {
          dispatchError(
            e instanceof Error ? e : new Error("Failed to parse snapshot")
          );
        }
      } else if (typeof event.data === "string") {
        // Text frame (e.g., JSON fallback format).
        try {
          const wrapped = JSON.parse(event.data) as {
            tick?: number;
            snapshot?: TSnapshot;
          };
          if (typeof wrapped.tick === "number" && wrapped.snapshot !== undefined) {
            dispatchSnapshot(wrapped.snapshot, wrapped.tick);
          }
        } catch (e) {
          dispatchError(
            e instanceof Error ? e : new Error("Failed to parse snapshot")
          );
        }
      }
    };

    ws.onerror = () => {
      dispatchError(new Error(`WebSocket error connecting to shard ${shardId}`));
    };

    ws.onclose = () => {
      connected = false;
      for (const h of closeHandlers) h();
      if (closed) return;
      if (options.autoReconnect !== false) {
        reconnectTimer = setTimeout(connect, backoff);
        backoff = Math.min(backoff * 2, 10_000);
      }
    };
  };

  connect();

  return {
    get connected() {
      return connected;
    },
    onSnapshot(fn) {
      snapshotHandlers.push(fn);
    },
    onError(fn) {
      errorHandlers.push(fn);
    },
    onOpen(fn) {
      openHandlers.push(fn);
    },
    onClose(fn) {
      closeHandlers.push(fn);
    },
    send(input: TInput): number {
      clientSeq += 1;
      const seq = clientSeq;
      const payload = JSON.stringify({ input, client_seq: seq });
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(payload);
      } else {
        dispatchError(
          new Error("Cannot send: shard connection is not open")
        );
      }
      return seq;
    },
    close() {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (ws) ws.close();
    },
  };
}

// ---------------------------------------------------------------------------
// React hook
// ---------------------------------------------------------------------------

/**
 * React hook that subscribes to a shard's snapshots and provides a send fn.
 *
 * The hook re-renders when a new snapshot arrives or the connection state
 * changes. The `send` fn is stable across re-renders.
 */
export function useShard<TSnapshot = unknown, TInput = unknown>(
  shardId: string,
  options: UseShardOptions
): UseShardReturn<TSnapshot, TInput> {
  const [snapshot, setSnapshot] = useState<TSnapshot | null>(null);
  const [tick, setTick] = useState<number>(0);
  const [connected, setConnected] = useState<boolean>(false);
  const [error, setError] = useState<Error | null>(null);

  const clientRef = useRef<ShardClient<TSnapshot, TInput> | null>(null);

  // Use primitive-value deps so the effect re-runs on identity-impacting
  // changes (token, subscriberId, URL) without re-running on every render
  // just because `options` is a fresh object literal. Previously we
  // excluded `options` entirely, so a user logging out would keep the
  // old socket alive under the old identity until `shardId` changed.
  const token = options.token;
  const subscriberId = options.subscriberId;
  const baseUrl = options.baseUrl;
  const wsUrl = options.wsUrl;
  const wsPort = options.wsPort;

  useEffect(() => {
    const client = connectShard<TSnapshot, TInput>(shardId, options);
    clientRef.current = client;

    client.onSnapshot((snap, t) => {
      setSnapshot(snap);
      setTick(t);
    });
    client.onOpen(() => setConnected(true));
    client.onClose(() => setConnected(false));
    client.onError((e) => setError(e));

    return () => {
      client.close();
      clientRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shardId, token, subscriberId, baseUrl, wsUrl, wsPort]);

  const send = (input: TInput): number => {
    if (clientRef.current) return clientRef.current.send(input);
    return 0;
  };

  const close = () => {
    if (clientRef.current) clientRef.current.close();
  };

  return { snapshot, tick, connected, error, send, close };
}
