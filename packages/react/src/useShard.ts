"use client";

/**
 * useShard — React hook for real-time sharded simulations (games, MMO zones, etc.).
 *
 * Connects to a Pylon shard over WebSocket, receives snapshots as they
 * arrive, and sends inputs upstream. It re-renders on every frame, so it
 * suits small, turn-based, or UI-only use; a game's render loop should use
 * `connectShardGame` from `@pylonsync/realtime`.
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
  connectShard,
  type EntityTable,
  type ShardClient,
  type ShardConnectOptions,
  type ShardInputRejection,
} from "@pylonsync/realtime";

// The connection itself lives in `@pylonsync/realtime`; these names stay
// exported from here for existing imports.
export { connectShard };
export type { ShardClient };

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface UseShardOptions extends ShardConnectOptions {
  /** Unused: a shard connection is always a WebSocket. */
  sseFallback?: boolean;
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
  /** Send an input to the shard. Returns a client sequence number, or 0
   *  when the connection is not open. */
  send: (input: TInput) => number;
  /** Close the connection early. */
  close: () => void;
  /**
   * For a shard that replicates entities: the entities this client has
   * been sent, updated in place. The hook re-renders once per applied frame
   * (`entitiesVersion` changes). A render loop should use
   * `connectShardGame` from `@pylonsync/realtime` instead, which draws
   * entities between frames without re-rendering.
   */
  entities: EntityTable | null;
  entitiesVersion: number;
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
  // A ticket function is called on each connection attempt; a new function
  // on each render must not reconnect, so the effect reads the latest one.
  const ticketRef = useRef(options.ticket);
  ticketRef.current = options.ticket;
  const ticket = typeof options.ticket === "function" ? "function" : options.ticket;
  const subscriberId = options.subscriberId;
  const baseUrl = options.baseUrl;
  const wsUrl = options.wsUrl;
  const wsPort = options.wsPort;

  useEffect(() => {
    const client = connectShard<TSnapshot, TInput>(shardId, {
      ...options,
      ticket:
        typeof options.ticket === "function"
          ? () => {
              const current = ticketRef.current;
              return typeof current === "function" ? current() : (current ?? "");
            }
          : options.ticket,
    });
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
