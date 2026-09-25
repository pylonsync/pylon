/**
 * A shard connection with no framework: one WebSocket, reconnected with
 * backoff, that decodes frames, applies replication frames to an
 * `EntityTable`, and sends inputs. `useShard` in `@pylonsync/react` and
 * `connectShardGame` build on it.
 */

import { ShardClock } from "./clock";
import { EntityTable, type ReplicationSummary } from "./replication";
import {
  SHARD_PROTOCOL_VERSION,
  ShardFrameKind,
  decodeShardPayload,
  decodeShardRejection,
  encodeShardInput,
  parseShardFrame,
  type ShardInputRejection,
  type ShardPayloadDecoder,
  type ShardTransferNotice,
} from "./wire";

export interface ShardConnectOptions {
  /** Subscriber ID (usually the logged-in user ID). Required for multiplayer. */
  subscriberId: string;
  /**
   * Auth token. Sent as a `bearer.<token>` WebSocket subprotocol, which
   * keeps it out of URLs (proxy logs, devtools, and error telemetry record
   * URLs, not subprotocols).
   */
  token?: string;
  /**
   * Shard ticket from a server function (`ctx.shards.ticket(...)`). Sent as
   * a `ticket.<ticket>` WebSocket subprotocol. The shard checks it names
   * this shard and `subscriberId`, and passes its claims to the game's
   * authorization hooks.
   *
   * Tickets expire. Pass a function to get a new one for each connection
   * attempt, so a reconnect after the expiry still gets in. It receives the
   * shard the client is connecting to, which changes after a transfer.
   */
  ticket?: string | ((shardId: string) => string | Promise<string>);
  /** Host (and port) of the Pylon server. Defaults to `window.location.host`. */
  baseUrl?: string;
  /**
   * Connect to the dedicated shard port instead of `/shard` on the main
   * port (the dedicated port is the HTTP port + 3, e.g. 4324).
   */
  wsPort?: number;
  /**
   * Explicit WebSocket URL. Overrides baseUrl/wsPort. After a transfer its
   * `shard` query parameter is replaced with the new shard.
   */
  wsUrl?: string;
  /** Reconnect on unexpected close (default: true). */
  autoReconnect?: boolean;
  /** First reconnect delay in ms (default 500; doubles to at most 10 000). */
  reconnectBackoffMs?: number;
  /**
   * Decoder for a payload codec the client does not know: bincode (`2`) or
   * a game's own codec (`3`). JSON and MessagePack are built in.
   */
  decode?: ShardPayloadDecoder;
  /** The shard's tick rate, when known; otherwise the clock measures it. */
  tickRate?: number;
  /** Monotonic time in ms. Default `performance.now()`. */
  now?: () => number;
}

export interface ShardClient<TSnapshot = unknown, TInput = unknown> {
  /** `ack` is the highest `send()` sequence number the shard has processed. */
  onSnapshot: (fn: (snapshot: TSnapshot, tick: number, ack: number) => void) => void;
  /** Called when the shard refuses an input (see `ShardInputRejection.code`). */
  onInputRejected: (fn: (rejection: ShardInputRejection) => void) => void;
  /**
   * Called when the server moved this subscriber to another shard (a zone
   * line, a dungeon). The client reconnects there on its own, with the
   * ticket the server sent; `shardId` changes before the handlers run.
   */
  onTransfer: (fn: (shardId: string, from: string) => void) => void;
  /** The shard the client is connected (or connecting) to. */
  readonly shardId: string;
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
  /** The shard's current tick, estimated from frame arrivals. */
  readonly clock: ShardClock;
  /** The tick of the last frame, or -1. */
  readonly tick: number;
  /** The ack of the last frame. */
  readonly ack: number;
  /**
   * Smoothed time from sending an input to the first frame that
   * acknowledges it (ms), or null before one has. It includes the wait for
   * the shard's next tick.
   */
  readonly rttMs: number | null;
  onError: (fn: (err: Error) => void) => void;
  onOpen: (fn: () => void) => void;
  onClose: (fn: () => void) => void;
  /**
   * Send an input. Returns its sequence number, which later frames
   * acknowledge, or 0 when the connection is not open (the input is not
   * sent, and `onError` hears why).
   */
  send: (input: TInput) => number;
  close: () => void;
  readonly connected: boolean;
}

/** Input send times kept for the round-trip estimate. */
const MAX_TIMED = 256;

/**
 * Connect to a shard without React. Returns a client you can wire into any
 * framework or render loop.
 */
export function connectShard<TSnapshot = unknown, TInput = unknown>(
  shardId: string,
  options: ShardConnectOptions,
): ShardClient<TSnapshot, TInput> {
  const now = options.now ?? (() => performance.now());
  let currentShard = shardId;
  // The ticket a transfer frame carried. Used until the new shard sends a
  // frame (then a ticket function takes over), and for good with a fixed
  // `ticket`, which names the old shard.
  let transferTicket: string | null = null;
  let ws: WebSocket | null = null;
  let clientSeq = 0;
  let closed = false;
  let connected = false;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let transferring = false;
  let backoff = options.reconnectBackoffMs ?? 500;
  // The shard's codec, learned from the first frame. Until then inputs go
  // as JSON text, which every shard accepts.
  let codec: number | null = null;
  let lastTick = -1;
  let lastAck = 0;
  let rttMs: number | null = null;
  // Send time per unacknowledged sequence number, oldest first.
  const sentAt = new Map<number, number>();
  const clock = new ShardClock({ tickRate: options.tickRate });

  const snapshotHandlers: Array<(s: TSnapshot, t: number, ack: number) => void> = [];
  const rejectionHandlers: Array<(r: ShardInputRejection) => void> = [];
  const transferHandlers: Array<(shard: string, from: string) => void> = [];
  const replicationHandlers: Array<
    (entities: EntityTable, summary: ReplicationSummary, tick: number, ack: number) => void
  > = [];
  const entities = new EntityTable();
  const errorHandlers: Array<(e: Error) => void> = [];
  const openHandlers: Array<() => void> = [];
  const closeHandlers: Array<() => void> = [];

  const dispatchError = (err: Error) => {
    for (const h of errorHandlers) h(err);
  };

  const buildWsUrl = (): string => {
    if (options.wsUrl) {
      if (currentShard === shardId) return options.wsUrl;
      const url = new URL(options.wsUrl);
      url.searchParams.set("shard", currentShard);
      return url.toString();
    }
    const proto =
      typeof window !== "undefined" && window.location.protocol === "https:" ? "wss" : "ws";
    // Only shard id + subscriber id land in the URL: routing metadata, not
    // credentials.
    const params = new URLSearchParams({
      shard: currentShard,
      sid: options.subscriberId,
      v: String(SHARD_PROTOCOL_VERSION),
    });
    if (options.wsPort !== undefined) {
      const hostname = (
        options.baseUrl ?? (typeof window !== "undefined" ? window.location.hostname : "localhost")
      ).replace(/:\d+$/, "");
      return `${proto}://${hostname}:${options.wsPort}/?${params.toString()}`;
    }
    // Default: `/shard` on the page's own origin, which any proxy that
    // forwards WebSocket upgrades on 443 already reaches.
    const host =
      options.baseUrl || (typeof window !== "undefined" ? window.location.host : "localhost:4321");
    return `${proto}://${host}/shard?${params.toString()}`;
  };

  /** Round-trip samples for every input `ack` covers. */
  const timeAcks = (ack: number, at: number) => {
    for (const [seq, t] of sentAt) {
      if (seq > ack) break;
      const sample = at - t;
      rttMs = rttMs === null ? sample : rttMs + (sample - rttMs) / 8;
      sentAt.delete(seq);
    }
  };

  const scheduleReconnect = () => {
    if (closed || options.autoReconnect === false) return;
    reconnectTimer = setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 10_000);
  };

  const connect = () => {
    if (closed) return;
    if (transferTicket !== null) {
      open(transferTicket);
      return;
    }
    const source = options.ticket;
    if (typeof source !== "function") {
      open(source);
      return;
    }
    let ticket: string | Promise<string>;
    try {
      ticket = source(currentShard);
    } catch (e) {
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      scheduleReconnect();
      return;
    }
    if (typeof ticket === "string") {
      open(ticket);
      return;
    }
    ticket.then(
      (t) => {
        if (!closed) open(t);
      },
      (e) => {
        dispatchError(e instanceof Error ? e : new Error(String(e)));
        scheduleReconnect();
      },
    );
  };

  const open = (ticket: string | undefined) => {
    const url = buildWsUrl();
    try {
      // Subprotocol values must be RFC 6455 tokens; encode them so spaces
      // and punctuation do not break the handshake.
      const protocols: string[] = [];
      if (options.token) protocols.push(`bearer.${encodeURIComponent(options.token)}`);
      if (ticket) protocols.push(`ticket.${encodeURIComponent(ticket)}`);
      ws = protocols.length ? new WebSocket(url, protocols) : new WebSocket(url);
    } catch (e) {
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      connected = true;
      // Inputs sent on the old connection are never acknowledged on this
      // one (acks restart with the connection).
      sentAt.clear();
      for (const h of openHandlers) h();
    };

    ws.onmessage = (event) => {
      if (!(event.data instanceof ArrayBuffer)) return;
      const at = now();
      try {
        const frame = parseShardFrame(event.data);
        if (frame.kind === ShardFrameKind.Transfer) {
          const notice = decodeShardPayload(frame.codec, frame.payload) as ShardTransferNotice;
          const from = currentShard;
          currentShard = notice.shard;
          transferTicket = notice.ticket;
          // A new shard: its ticks, acks, and entities start over.
          clock.reset();
          entities.clear();
          lastTick = -1;
          lastAck = 0;
          sentAt.clear();
          codec = null;
          backoff = options.reconnectBackoffMs ?? 500;
          for (const h of transferHandlers) h(currentShard, from);
          // The server closes this connection; reconnect at once.
          transferring = true;
          return;
        }
        // The new shard answered: a ticket function gives the next tickets.
        if (typeof options.ticket === "function") transferTicket = null;
        if (frame.kind === ShardFrameKind.Replication || frame.kind === ShardFrameKind.Snapshot) {
          // Only the per-tick frames: a rejection can go out before its
          // tick's frame is built, and would make the clock run early.
          clock.observe(frame.tick, at);
          lastTick = frame.tick;
          lastAck = frame.ack;
          timeAcks(frame.ack, at);
        }
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
          const snapshot = decodeShardPayload(frame.codec, frame.payload, options.decode) as TSnapshot;
          backoff = options.reconnectBackoffMs ?? 500;
          for (const h of snapshotHandlers) h(snapshot, frame.tick, frame.ack);
        } else if (frame.kind === ShardFrameKind.InputRejected) {
          const rejection = decodeShardRejection(frame.codec, frame.payload, options.decode);
          if (rejection.clientSeq !== null) sentAt.delete(rejection.clientSeq);
          for (const h of rejectionHandlers) h(rejection);
        }
      } catch (e) {
        dispatchError(e instanceof Error ? e : new Error("Failed to decode shard frame"));
      }
    };

    ws.onerror = () => {
      dispatchError(new Error(`WebSocket error connecting to shard ${currentShard}`));
    };

    ws.onclose = () => {
      connected = false;
      for (const h of closeHandlers) h();
      if (transferring && !closed) {
        transferring = false;
        reconnectTimer = setTimeout(connect, 0);
        return;
      }
      scheduleReconnect();
    };
  };

  connect();

  return {
    get connected() {
      return connected;
    },
    get shardId() {
      return currentShard;
    },
    get entities() {
      return entities;
    },
    get clock() {
      return clock;
    },
    get tick() {
      return lastTick;
    },
    get ack() {
      return lastAck;
    },
    get rttMs() {
      return rttMs;
    },
    onSnapshot(fn) {
      snapshotHandlers.push(fn);
    },
    onInputRejected(fn) {
      rejectionHandlers.push(fn);
    },
    onTransfer(fn) {
      transferHandlers.push(fn);
    },
    onReplication(fn) {
      replicationHandlers.push(fn);
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
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        dispatchError(new Error("Cannot send: shard connection is not open"));
        return 0;
      }
      clientSeq += 1;
      ws.send(encodeShardInput(codec, input, clientSeq));
      sentAt.set(clientSeq, now());
      if (sentAt.size > MAX_TIMED) sentAt.delete(sentAt.keys().next().value as number);
      return clientSeq;
    },
    close() {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (ws) ws.close();
    },
  };
}
