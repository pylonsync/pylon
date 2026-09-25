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
import {
  EntityTable,
  SHARD_PROTOCOL_VERSION,
  ShardFrameKind,
  decodeShardPayload,
  decodeShardRejection,
  encodeShardInput,
  parseShardFrame,
  type ReplicationSummary,
  type ShardInputRejection,
  type ShardPayloadDecoder,
} from "@pylonsync/realtime";

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
  /**
   * Shard ticket from a server function (`ctx.shards.ticket(...)`). Sent as
   * a `ticket.<ticket>` WebSocket subprotocol. The shard checks it names
   * this shard and `subscriberId`, and passes its claims to the game's
   * authorization hooks.
   */
  ticket?: string;
  /** Host (and port) of the Pylon server. Defaults to `window.location.host`. */
  baseUrl?: string;
  /**
   * Connect to the dedicated shard port instead of `/shard` on the main
   * port (the dedicated port is the HTTP port + 3, e.g. 4324).
   */
  wsPort?: number;
  /** Explicit WebSocket URL. Overrides baseUrl/wsPort. */
  wsUrl?: string;
  /** If true, falls back to SSE + HTTP POST if WebSocket fails (default: true). */
  sseFallback?: boolean;
  /** Reconnect on unexpected close (default: true). */
  autoReconnect?: boolean;
  /** Reconnect backoff in ms (default: starts at 500, maxes at 10_000). */
  reconnectBackoffMs?: number;
  /**
   * Decoder for a payload codec the client does not know: bincode (`2`) or
   * a game's own codec (`3`). JSON and MessagePack are built in.
   */
  decode?: ShardPayloadDecoder;
}

export interface UseShardReturn<TSnapshot = unknown, TInput = unknown> {
  snapshot: TSnapshot | null;
  tick: number;
  /**
   * The highest `send()` sequence number the shard has processed (applied
   * or rejected) as of `snapshot`. Drop local predictions up to it and
   * replay the rest on top of `snapshot`.
   */
  ack: number;
  /** The most recent input the shard refused, if any. */
  lastRejection: ShardInputRejection | null;
  connected: boolean;
  error: Error | null;
  /** Send an input to the shard. Returns a client sequence number. */
  send: (input: TInput) => number;
  /** Close the connection early. */
  close: () => void;
  /**
   * For a shard that replicates entities: the entities this client has
   * been sent, updated in place. The hook re-renders once per applied frame
   * (`entitiesVersion` changes); a render loop can read it directly.
   */
  entities: EntityTable | null;
  entitiesVersion: number;
}

// ---------------------------------------------------------------------------
// Low-level client (no React)
// ---------------------------------------------------------------------------

export interface ShardClient<TSnapshot = unknown, TInput = unknown> {
  /** `ack` is the highest `send()` sequence number the shard has processed. */
  onSnapshot: (fn: (snapshot: TSnapshot, tick: number, ack: number) => void) => void;
  /** Called when the shard refuses an input (see `ShardInputRejection.code`). */
  onInputRejected: (fn: (rejection: ShardInputRejection) => void) => void;
  /**
   * For a shard that replicates entities: called after each frame is
   * applied to `entities`, with what it changed.
   */
  onReplication: (
    fn: (entities: EntityTable, summary: ReplicationSummary, tick: number, ack: number) => void,
  ) => void;
  /**
   * The entities a replicating shard has sent this client. Read it each
   * frame (a render loop); it changes in place as frames arrive.
   */
  readonly entities: EntityTable;
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
  // The shard's codec, learned from the first frame. Until then inputs go
  // as JSON text, which every shard accepts.
  let codec: number | null = null;

  const snapshotHandlers: Array<(s: TSnapshot, t: number, ack: number) => void> = [];
  const rejectionHandlers: Array<(r: ShardInputRejection) => void> = [];
  const replicationHandlers: Array<
    (entities: EntityTable, summary: ReplicationSummary, tick: number, ack: number) => void
  > = [];
  const entities = new EntityTable();
  const errorHandlers: Array<(e: Error) => void> = [];
  const openHandlers: Array<() => void> = [];
  const closeHandlers: Array<() => void> = [];

  const dispatchSnapshot = (snapshot: TSnapshot, tick: number, ack: number) => {
    for (const h of snapshotHandlers) h(snapshot, tick, ack);
  };
  const dispatchError = (err: Error) => {
    for (const h of errorHandlers) h(err);
  };

  const buildWsUrl = (): string => {
    if (options.wsUrl) return options.wsUrl;
    const proto =
      typeof window !== "undefined" && window.location.protocol === "https:"
        ? "wss"
        : "ws";
    // Only shard id + subscriber id land in the URL — these are routing
    // metadata, not credentials.
    const params = new URLSearchParams({
      shard: shardId,
      sid: options.subscriberId,
      v: String(SHARD_PROTOCOL_VERSION),
    });
    if (options.wsPort !== undefined) {
      const hostname =
        (options.baseUrl ?? (typeof window !== "undefined" ? window.location.hostname : "localhost"))
          .replace(/:\d+$/, "");
      return `${proto}://${hostname}:${options.wsPort}/?${params.toString()}`;
    }
    // Default: `/shard` on the page's own origin, which any proxy that
    // forwards WebSocket upgrades on 443 already reaches.
    const host =
      options.baseUrl || (typeof window !== "undefined" ? window.location.host : "localhost:4321");
    return `${proto}://${host}/shard?${params.toString()}`;
  };

  const connect = () => {
    if (closed) return;
    const url = buildWsUrl();
    try {
      // The bearer token rides on the WebSocket subprotocol header so it
      // doesn't get captured by every proxy / devtools pane that logs URLs.
      // Subprotocol values must be a token per RFC 6455; encode the bearer
      // so spaces/punctuation don't break the handshake.
      const protocols: string[] = [];
      if (options.token) protocols.push(`bearer.${encodeURIComponent(options.token)}`);
      if (options.ticket) protocols.push(`ticket.${encodeURIComponent(options.ticket)}`);
      ws = protocols.length ? new WebSocket(url, protocols) : new WebSocket(url);
    } catch (e) {
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      connected = true;
      for (const h of openHandlers) h();
    };

    ws.onmessage = (event) => {
      if (!(event.data instanceof ArrayBuffer)) return;
      try {
        const frame = parseShardFrame(event.data);
        if (frame.kind === ShardFrameKind.Replication) {
          let summary: ReplicationSummary;
          try {
            summary = entities.apply(frame.payload);
          } catch (e) {
            // Out of sync with the server. Reconnecting gets a full baseline.
            dispatchError(e instanceof Error ? e : new Error(String(e)));
            entities.clear();
            ws?.close();
            return;
          }
          // A frame applied: the connection works, so the next reconnect
          // starts from the short delay again. (Resetting on open would
          // retry a frame that always fails every 500 ms forever.)
          backoff = options.reconnectBackoffMs ?? 500;
          for (const h of replicationHandlers) h(entities, summary, frame.tick, frame.ack);
          return;
        }
        // The replication codec byte names the frame format, not the
        // shard's input codec, so only other frames set it.
        codec = frame.codec;
        if (frame.kind === ShardFrameKind.Snapshot) {
          const snapshot = decodeShardPayload(
            frame.codec,
            frame.payload,
            options.decode,
          ) as TSnapshot;
          backoff = options.reconnectBackoffMs ?? 500;
          dispatchSnapshot(snapshot, frame.tick, frame.ack);
        } else if (frame.kind === ShardFrameKind.InputRejected) {
          const rejection = decodeShardRejection(frame.codec, frame.payload, options.decode);
          for (const h of rejectionHandlers) h(rejection);
        }
      } catch (e) {
        dispatchError(e instanceof Error ? e : new Error("Failed to decode shard frame"));
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
    onInputRejected(fn) {
      rejectionHandlers.push(fn);
    },
    onReplication(fn) {
      replicationHandlers.push(fn);
    },
    get entities() {
      return entities;
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
      const payload = encodeShardInput(codec, input, seq);
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
  const [ack, setAck] = useState<number>(0);
  const [lastRejection, setLastRejection] = useState<ShardInputRejection | null>(null);
  const [connected, setConnected] = useState<boolean>(false);
  const [error, setError] = useState<Error | null>(null);
  const [entitiesVersion, setEntitiesVersion] = useState<number>(0);

  const clientRef = useRef<ShardClient<TSnapshot, TInput> | null>(null);

  // Use primitive-value deps so the effect re-runs on identity-impacting
  // changes (token, subscriberId, URL) without re-running on every render
  // just because `options` is a fresh object literal. Previously we
  // excluded `options` entirely, so a user logging out would keep the
  // old socket alive under the old identity until `shardId` changed.
  const token = options.token;
  const ticket = options.ticket;
  const subscriberId = options.subscriberId;
  const baseUrl = options.baseUrl;
  const wsUrl = options.wsUrl;
  const wsPort = options.wsPort;

  useEffect(() => {
    const client = connectShard<TSnapshot, TInput>(shardId, options);
    clientRef.current = client;

    client.onSnapshot((snap, t, a) => {
      setSnapshot(snap);
      setTick(t);
      setAck(a);
    });
    client.onInputRejected((r) => setLastRejection(r));
    client.onReplication((_table, _summary, t, a) => {
      setTick(t);
      setAck(a);
      setEntitiesVersion((v) => v + 1);
    });
    client.onOpen(() => setConnected(true));
    client.onClose(() => setConnected(false));
    client.onError((e) => setError(e));

    return () => {
      client.close();
      clientRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shardId, token, ticket, subscriberId, baseUrl, wsUrl, wsPort]);

  const send = (input: TInput): number => {
    if (clientRef.current) return clientRef.current.send(input);
    return 0;
  };

  const close = () => {
    if (clientRef.current) clientRef.current.close();
  };

  return {
    snapshot,
    tick,
    ack,
    lastRejection,
    connected,
    error,
    send,
    close,
    entities: clientRef.current?.entities ?? null,
    entitiesVersion,
  };
}
