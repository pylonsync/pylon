/**
 * A shard connection with no framework: one WebSocket or WebTransport
 * session, reconnected with backoff, that decodes frames, applies
 * replication frames to an `EntityTable`, and sends inputs. `useShard` in
 * `@pylonsync/react` and `connectShardGame` build on it.
 */

import { ShardClock } from "./clock";
import { EntityTable, isFullFrame, readDatagramHeader, type ReplicationSummary } from "./replication";
import {
  SHARD_PROTOCOL_VERSION,
  ShardFrameKind,
  WEBTRANSPORT_CLOSE,
  StreamFrames,
  decodeWebTransportInfo,
  type WebTransportInfo,
  encodeDatagramAckBatches,
  encodeWebTransportHello,
  encodeWebTransportInput,
  decodeShardPayload,
  decodeShardRejection,
  encodeShardInput,
  parseShardFrame,
  type ShardInputRejection,
  type ShardPayloadDecoder,
  type ShardTransferNotice,
} from "./wire";

/**
 * How the client reaches the shard.
 *
 * - `"websocket"`: a WebSocket (TCP). Works everywhere.
 * - `"webtransport"`: a WebTransport session (QUIC over UDP). Entity
 *   updates come as datagrams, so a lost packet delays only itself. Needs
 *   the browser API and an app that serves WebTransport
 *   (`PYLON_WEBTRANSPORT_PORT`); fails otherwise.
 * - `"auto"`: WebTransport when the browser and the app support it, else a
 *   WebSocket. After a WebTransport session fails to open (UDP blocked, an
 *   old browser), the client uses WebSockets for the rest of its life.
 */
export type ShardTransport = "websocket" | "webtransport" | "auto";

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
  /** Default `"websocket"`. See `ShardTransport`. */
  transport?: ShardTransport;
  /**
   * Where to fetch the WebTransport URL and certificate hashes. Default
   * `/_pylon/shard/webtransport` on `baseUrl` (or the page's host), or on
   * the host of `wsUrl` when only that is set.
   */
  webTransportInfoUrl?: string;
  /**
   * How long a WebTransport session may take to open before `"auto"`
   * falls back to a WebSocket, in ms (default 3000).
   */
  webTransportTimeoutMs?: number;
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
   * When the shard itself moves to another machine (a deploy), the client
   * reconnects the same way, and these handlers are not called.
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
  /** The transport of the open (or last) connection, or null before one opened. */
  readonly transport: "websocket" | "webtransport" | null;
}

/** WebSocket close code 1008: the server refused the connection by policy. */
const POLICY_CLOSE = 1008;

/**
 * True when a shard ticket (`v1.<payload>.<signature>`, the payload
 * base64url JSON with `exp` in Unix seconds) expires within `marginSecs`.
 * A ticket that does not parse counts as not expired: the server decides.
 */
export function ticketExpired(ticket: string, nowMs = Date.now(), marginSecs = 5): boolean {
  const payload = ticket.split(".")[1];
  if (!payload) return false;
  try {
    const b64 = payload.replace(/-/g, "+").replace(/_/g, "/");
    const json = JSON.parse(atob(b64.padEnd(Math.ceil(b64.length / 4) * 4, "="))) as {
      exp?: unknown;
    };
    return typeof json.exp === "number" && json.exp * 1000 <= nowMs + marginSecs * 1000;
  } catch {
    return false;
  }
}

/** Input send times kept for the round-trip estimate. */
const MAX_TIMED = 256;

/** The open connection, over either transport. */
interface Link {
  readonly kind: "websocket" | "webtransport";
  /** True while the link can send. */
  readonly open: boolean;
  /** Send an input, as `encodeShardInput` made it. */
  send(input: string | Uint8Array): void;
  /** Send datagram acks: (datagram number, stream tick). WebTransport only. */
  ack(acks: Array<[number, number]>): void;
  /**
   * What the server said before it closed the session (a closing frame):
   * a WebTransport close from the server does not reach the browser's
   * `closed` with its code.
   */
  notice: { code: number; reason: string } | null;
  close(): void;
}

/** A tick's datagrams waiting until the tick is whole. */
interface PendingTick {
  parts: number;
  streamTick: number;
  ack: number;
  /** By datagram number (a duplicate replaces itself). */
  datagrams: Map<number, Uint8Array>;
}

/** A stream frame waiting until its tick is whole. */
interface PendingFrame {
  tick: number;
  payload: Uint8Array;
}

/**
 * How long a WebTransport session may go without a whole tick (ms). Past
 * that, its datagrams have stopped arriving: the client closes it. The
 * server sends at least one datagram every tick.
 */
const STALL_MS = 5000;

/** Ticks kept waiting at most; more means the datagrams have stopped. */
const MAX_PENDING_TICKS = 100;

/** The parts of the browser's `WebTransport` the client uses. */
interface WebTransportSession {
  readonly ready: Promise<void>;
  readonly closed: Promise<{ closeCode?: number; reason?: string } | undefined>;
  readonly datagrams: {
    readonly readable: ReadableStream<Uint8Array>;
    /** The largest datagram the session can send now. */
    readonly maxDatagramSize?: number;
    readonly writable?: WritableStream<Uint8Array>;
    createWritable?: () => WritableStream<Uint8Array>;
  };
  createBidirectionalStream(): Promise<{
    readable: ReadableStream<Uint8Array>;
    writable: WritableStream<Uint8Array>;
  }>;
  close(info?: { closeCode?: number; reason?: string }): void;
}

type WebTransportConstructor = new (
  url: string,
  options?: {
    serverCertificateHashes?: Array<{ algorithm: "sha-256"; value: Uint8Array }>;
    requireUnreliable?: boolean;
  },
) => WebTransportSession;

/**
 * Why a WebTransport session did not open. `unsupported`: the browser has
 * no API, or the app does not serve WebTransport. `transient`: the
 * endpoint info could not be fetched. `failed`: the session did not open.
 */
class WebTransportUnavailable extends Error {
  constructor(
    readonly reason: "unsupported" | "transient" | "failed",
    message: string,
  ) {
    super(message);
  }
}

const webTransportApi = () =>
  (globalThis as { WebTransport?: WebTransportConstructor }).WebTransport;

const errorMessage = (e: unknown) => (e instanceof Error ? e.message : String(e));

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
  let link: Link | null = null;
  // Each WebTransport attempt's number; a newer attempt or a close makes an
  // older one stand down.
  let attempts = 0;
  // `"auto"` after WebTransport failed to open: WebSockets from now on.
  let webSocketOnly = false;
  // Stops the WebTransport attempt in progress (its endpoint request).
  let opening: AbortController | null = null;
  // The newest tick and ack this link has seen. Over WebTransport, stream
  // frames and datagrams can arrive out of order.
  let linkTick = -1;
  let linkAck = 0;
  // The tick handlers last got: they see ticks in order.
  let reportedTick = -1;
  // Over WebTransport: the newest tick whose state the table holds whole
  // (all its datagrams and stream frames applied), and the datagrams of
  // newer ticks that are not whole yet (see `pylon_replication::datagram`).
  let wholeTick = -1;
  const pending = new Map<number, PendingTick>();
  // Stream frames (not full) that wait with their tick's datagrams, in
  // order, and the tick of the newest one.
  const pendingStream: PendingFrame[] = [];
  let receivedStreamTick = -1;
  // When the table last held a whole tick (or the session opened).
  let lastWholeAt = 0;
  // Whether this link replicates entities. Only then does the server send
  // datagrams every tick; a snapshot shard sends none.
  let replicating = false;
  let lastKind: Link["kind"] | null = null;
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
    const source = options.ticket;
    // A transfer's ticket that expired before the client got through: a
    // ticket function gives a new one; a fixed ticket still works for the
    // shard it names.
    if (transferTicket !== null && ticketExpired(transferTicket)) {
      transferTicket = null;
      if (typeof source !== "function" && currentShard !== shardId) {
        dispatchError(
          new Error(
            `the ticket for shard ${currentShard} expired before the client reconnected; pass a ticket function to get new ones`,
          ),
        );
        closed = true;
        return;
      }
    }
    if (transferTicket !== null) {
      open(transferTicket);
      return;
    }
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

  /** The link opened: its frames start over. */
  const began = (l: Link) => {
    link = l;
    lastKind = l.kind;
    connected = true;
    linkTick = -1;
    linkAck = 0;
    reportedTick = -1;
    wholeTick = -1;
    pending.clear();
    pendingStream.length = 0;
    receivedStreamTick = -1;
    lastWholeAt = now();
    replicating = false;
    // Inputs sent on the old connection are never acknowledged on this
    // one (acks restart with the connection).
    sentAt.clear();
    for (const h of openHandlers) h();
  };

  /** A per-tick frame or datagram for `tick` arrived. */
  const observeTick = (tick: number, at: number) => {
    // The first arrival for a tick times the clock best; a late stream
    // frame after a newer datagram would move it back.
    if (tick > linkTick) {
      clock.observe(tick, at);
      linkTick = tick;
    }
    lastTick = linkTick;
  };

  /** The table now holds the shard's state after the inputs up to `ack`. */
  const takeAck = (ack: number, at: number) => {
    if (ack > linkAck) linkAck = ack;
    lastAck = linkAck;
    timeAcks(linkAck, at);
  };

  const report = (summary: ReplicationSummary, tick: number) => {
    reportedTick = Math.max(reportedTick, tick);
    for (const h of replicationHandlers) h(entities, summary, reportedTick, lastAck);
  };

  /**
   * Apply everything buffered up to the newest whole tick, oldest tick
   * first (each tick's stream frames, then its datagrams), then take that
   * tick's ack, tell the handlers once, and ack the datagrams. A tick is
   * whole when all its datagrams are here and so are the stream frames
   * sent by then. Until then its changes wait, so the handlers never see a
   * table newer than the ack they get with it (prediction relies on that).
   */
  const applyWhole = (from: Link, at: number) => {
    try {
      applyWholeTick(from, at);
    } catch (e) {
      // Out of sync with the server. Reconnecting gets a full baseline.
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      entities.clear();
      from.close();
    }
  };

  const applyWholeTick = (from: Link, at: number) => {
    let whole = -1;
    for (const [tick, p] of pending) {
      if (tick > whole && p.datagrams.size === p.parts && receivedStreamTick >= p.streamTick) {
        whole = tick;
      }
    }
    if (whole < 0) return;
    const ack = (pending.get(whole) as PendingTick).ack;
    const ticks = new Set<number>();
    for (const t of pending.keys()) if (t <= whole) ticks.add(t);
    for (const f of pendingStream) if (f.tick <= whole) ticks.add(f.tick);
    const spawned = new Set<number>();
    const despawned = new Set<number>();
    const updated = new Set<number>();
    const acks: Array<[number, number]> = [];
    for (const t of [...ticks].sort((a, b) => a - b)) {
      while (pendingStream.length > 0 && pendingStream[0].tick === t) {
        const f = pendingStream.shift() as PendingFrame;
        const s = entities.apply(f.payload, f.tick);
        for (const id of s.despawned) {
          despawned.add(id);
          spawned.delete(id);
          updated.delete(id);
        }
        for (const id of s.spawned) spawned.add(id);
        for (const id of s.updated) updated.add(id);
      }
      const p = pending.get(t);
      if (!p) continue;
      pending.delete(t);
      for (const number of [...p.datagrams.keys()].sort((a, b) => a - b)) {
        const s = entities.applyDatagram(p.datagrams.get(number) as Uint8Array);
        for (const id of s.updated) updated.add(id);
        acks.push([s.frame, entities.streamTick]);
      }
    }
    for (const id of spawned) updated.delete(id);
    wholeTick = whole;
    lastWholeAt = at;
    takeAck(ack, at);
    backoff = options.reconnectBackoffMs ?? 500;
    report(
      { full: false, spawned: [...spawned], updated: [...updated], despawned: [...despawned] },
      whole,
    );
    if (acks.length > 0) from.ack(acks);
  };

  /**
   * Too many ticks without one whole: the datagrams stopped arriving (UDP
   * blocked partway, say). Close the session; `"auto"` uses WebSockets
   * from then on.
   */
  const stalled = (from: Link) => {
    if (
      from !== link ||
      !replicating ||
      (now() - lastWholeAt <= STALL_MS &&
        pending.size <= MAX_PENDING_TICKS &&
        pendingStream.length <= MAX_PENDING_TICKS)
    ) {
      return false;
    }
    dispatchError(new Error(`WebTransport to shard ${currentShard}: datagrams stopped arriving`));
    if ((options.transport ?? "websocket") === "auto") webSocketOnly = true;
    from.close();
    return true;
  };

  /** A frame from the server: a WebSocket message, or one from the WebTransport stream. */
  const onFrame = (from: Link, data: ArrayBuffer, at: number) => {
    if (from !== link) return;
    try {
      const frame = parseShardFrame(data);
      if (frame.kind === ShardFrameKind.Transfer) {
        const notice = decodeShardPayload(frame.codec, frame.payload) as ShardTransferNotice;
        const previous = currentShard;
        currentShard = notice.shard;
        transferTicket = notice.ticket;
        // A new shard, or this one started on another machine (a deploy):
        // its ticks, acks, and entities start over.
        clock.reset();
        entities.clear();
        lastTick = -1;
        lastAck = 0;
        sentAt.clear();
        codec = null;
        backoff = options.reconnectBackoffMs ?? 500;
        // The same shard on another machine is not a move for the app.
        if (currentShard !== previous) {
          for (const h of transferHandlers) h(currentShard, previous);
        }
        // The server closes this connection; reconnect at once.
        transferring = true;
        return;
      }
      if (frame.kind === ShardFrameKind.Closing) {
        // The server is about to close the session: keep why, and close
        // it from this side (see `Link.notice`).
        const notice = decodeShardPayload(frame.codec, frame.payload) as {
          code?: unknown;
          reason?: unknown;
        };
        from.notice = { code: Number(notice.code ?? 0), reason: String(notice.reason ?? "") };
        from.close();
        return;
      }
      // The new shard answered: a ticket function gives the next tickets.
      if (typeof options.ticket === "function") transferTicket = null;
      if (frame.kind === ShardFrameKind.Replication || frame.kind === ShardFrameKind.Snapshot) {
        // Only the per-tick frames: a rejection can go out before its
        // tick's frame is built, and would make the clock run early.
        observeTick(frame.tick, at);
      }
      if (frame.kind === ShardFrameKind.Replication) replicating = true;
      if (frame.kind === ShardFrameKind.Replication && from.kind === "webtransport" && !isFullFrame(frame.payload)) {
        // It waits with its tick's datagrams (see `applyWhole`).
        pendingStream.push({ tick: frame.tick, payload: frame.payload });
        receivedStreamTick = Math.max(receivedStreamTick, frame.tick);
        if (!stalled(from)) applyWhole(from, at);
        return;
      }
      if (frame.kind === ShardFrameKind.Replication) {
        let summary: ReplicationSummary;
        try {
          summary = entities.apply(frame.payload, frame.tick);
        } catch (e) {
          // Out of sync with the server. Reconnecting gets a full baseline.
          dispatchError(e instanceof Error ? e : new Error(String(e)));
          entities.clear();
          from.close();
          return;
        }
        // A full frame holds everything, so its ack describes the table.
        takeAck(frame.ack, at);
        if (from.kind === "webtransport") {
          // What was built before it describes a table that is gone.
          wholeTick = Math.max(wholeTick, frame.tick);
          lastWholeAt = at;
          receivedStreamTick = Math.max(receivedStreamTick, frame.tick);
          for (const t of [...pending.keys()]) if (t <= frame.tick) pending.delete(t);
          while (pendingStream.length > 0 && pendingStream[0].tick <= frame.tick) pendingStream.shift();
        }
        // A frame applied: the connection works, so the next reconnect
        // starts from the short delay again. (Resetting on open would
        // retry a frame that always fails every 500 ms forever.)
        backoff = options.reconnectBackoffMs ?? 500;
        report(summary, frame.tick);
        if (from.kind === "webtransport") applyWhole(from, at);
        return;
      }
      // The replication codec byte names the frame format, not the
      // shard's input codec, so only other frames set it.
      codec = frame.codec;
      if (frame.kind === ShardFrameKind.Snapshot) {
        takeAck(frame.ack, at);
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

  /**
   * A WebTransport datagram: entity updates for one tick. It waits until
   * its tick is whole (see `applyWhole`). One for a tick at or before the
   * newest whole tick is dropped unacked, as if lost: the server sends its
   * changes again.
   */
  const onDatagram = (from: Link, datagram: Uint8Array, at: number) => {
    if (from !== link) return;
    try {
      const h = readDatagramHeader(datagram);
      replicating = true;
      if (h.tick <= wholeTick || h.parts < 1) return;
      observeTick(h.tick, at);
      let p = pending.get(h.tick);
      if (!p) {
        p = { parts: h.parts, streamTick: h.streamTick, ack: h.ack, datagrams: new Map() };
        pending.set(h.tick, p);
      }
      p.datagrams.set(h.frame, datagram);
      if (!stalled(from)) applyWhole(from, at);
    } catch (e) {
      // Not a datagram this client can read: start over with a baseline.
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      entities.clear();
      from.close();
    }
  };

  /**
   * The link closed. `refusal` is the reason when the server refused the
   * credentials (WebSocket close 1008 or WebTransport code 1 with an
   * `unauthorized` reason).
   */
  const ended = (from: Link, refusal: string | null) => {
    if (from !== link) return;
    link = null;
    connected = false;
    for (const h of closeHandlers) h();
    if (transferring && !closed) {
      transferring = false;
      reconnectTimer = setTimeout(connect, 0);
      return;
    }
    // The server refused these credentials (an expired ticket, one signed
    // with an old secret, a session that ended). The same ones fail the
    // same way on every retry; only a ticket function can bring new ones.
    const sameCredentials = transferTicket !== null || typeof options.ticket !== "function";
    if (refusal !== null && sameCredentials && !closed) {
      dispatchError(
        new Error(
          `shard ${currentShard} refused the connection (${refusal}); pass a ticket function to get new tickets`,
        ),
      );
      closed = true;
      return;
    }
    scheduleReconnect();
  };

  const open = (ticket: string | undefined) => {
    const mode = options.transport ?? "websocket";
    // Without the browser API, `"auto"` goes straight to the WebSocket.
    if (mode === "auto" && typeof webTransportApi() !== "function") webSocketOnly = true;
    if (mode === "websocket" || (mode === "auto" && webSocketOnly)) {
      openWebSocket(ticket);
      return;
    }
    attempts += 1;
    const attempt = attempts;
    openWebTransport(ticket, attempt).catch((e: unknown) => {
      if (closed || attempt !== attempts) return;
      const reason = e instanceof WebTransportUnavailable ? e.reason : "failed";
      if (mode === "auto") {
        // The WebSocket works where WebTransport does not: UDP blocked, an
        // old browser, an app without it. A failed fetch of the endpoint
        // info may pass; the rest do not.
        if (reason !== "transient") webSocketOnly = true;
        openWebSocket(ticket);
        return;
      }
      dispatchError(new Error(`WebTransport to shard ${currentShard}: ${errorMessage(e)}`));
      if (reason === "unsupported") closed = true;
      else scheduleReconnect();
    });
  };

  const webTransportInfoUrl = (): string => {
    if (options.webTransportInfoUrl) return options.webTransportInfoUrl;
    const path = "/_pylon/shard/webtransport";
    if (!options.baseUrl && options.wsUrl && options.wsPort === undefined) {
      const url = new URL(options.wsUrl);
      const proto = url.protocol === "wss:" ? "https:" : "http:";
      return `${proto}//${url.host}${path}`;
    }
    const proto =
      typeof window !== "undefined" && window.location.protocol === "https:" ? "https" : "http";
    const host =
      options.baseUrl || (typeof window !== "undefined" ? window.location.host : "localhost:4321");
    return `${proto}://${host}${path}`;
  };

  const openWebTransport = async (ticket: string | undefined, attempt: number) => {
    const WT = webTransportApi();
    if (typeof WT !== "function") {
      throw new WebTransportUnavailable("unsupported", "this browser has no WebTransport");
    }
    // One deadline for the whole open: the endpoint request, its body,
    // the session, and its stream. The request stops at the deadline, or
    // when the client closes.
    const timeoutMs = options.webTransportTimeoutMs ?? 3000;
    const abort = new AbortController();
    opening = abort;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        abort.abort();
        reject(new WebTransportUnavailable("failed", `the session did not open in ${timeoutMs} ms`));
      }, timeoutMs);
    });
    timeout.catch(() => {});
    let wt: WebTransportSession | null = null;
    let stream: Awaited<ReturnType<WebTransportSession["createBidirectionalStream"]>>;
    try {
      let info: WebTransportInfo;
      try {
        const response = await Promise.race([
          fetch(webTransportInfoUrl(), { signal: abort.signal }),
          timeout,
        ]);
        if (response.status === 404) {
          throw new WebTransportUnavailable("unsupported", "the app does not serve WebTransport");
        }
        if (!response.ok) {
          throw new WebTransportUnavailable("transient", `fetching the endpoint: HTTP ${response.status}`);
        }
        info = decodeWebTransportInfo(await Promise.race([response.json(), timeout]));
      } catch (e) {
        if (e instanceof WebTransportUnavailable) throw e;
        // The app did not answer: the WebSocket may still work, and a later
        // attempt may reach the endpoint.
        throw new WebTransportUnavailable("transient", `fetching the endpoint: ${errorMessage(e)}`);
      }
      if (closed || attempt !== attempts) return;

      wt = new WT(info.url, {
        // Pins the server's self-signed certificate; none for a CA-signed one.
        ...(info.certHashes.length > 0
          ? { serverCertificateHashes: info.certHashes.map((value) => ({ algorithm: "sha-256" as const, value })) }
          : {}),
        requireUnreliable: true,
      });
      // `closed` rejects when the session never opens; that is handled below.
      wt.closed.catch(() => {});
      await Promise.race([wt.ready, timeout]);
      stream = await Promise.race([wt.createBidirectionalStream(), timeout]);
    } catch (e) {
      try {
        wt?.close();
      } catch {
        // Already closed.
      }
      throw e instanceof WebTransportUnavailable
        ? e
        : new WebTransportUnavailable("failed", `the session did not open: ${errorMessage(e)}`);
    } finally {
      clearTimeout(timer);
      if (opening === abort) opening = null;
    }
    // Set whenever the stream is.
    const session = wt as WebTransportSession | null;
    if (!session) return;
    if (closed || attempt !== attempts) {
      session.close();
      return;
    }

    const writer = stream.writable.getWriter();
    const datagrams = (session.datagrams.createWritable?.() ?? session.datagrams.writable)?.getWriter();
    if (!datagrams) {
      session.close();
      throw new WebTransportUnavailable("unsupported", "this browser cannot send WebTransport datagrams");
    }
    // A failed write shows up as the session closing.
    const ignore = () => {};
    writer
      .write(
        encodeWebTransportHello({
          shard: currentShard,
          sid: options.subscriberId,
          ...(ticket ? { ticket } : {}),
          ...(options.token ? { token: options.token } : {}),
        }),
      )
      .catch(ignore);
    let isOpen = true;
    const l: Link = {
      kind: "webtransport",
      get open() {
        return isOpen;
      },
      send(input) {
        writer.write(encodeWebTransportInput(input)).catch(ignore);
      },
      ack(acks) {
        // A datagram larger than the session allows never arrives.
        const max = session.datagrams.maxDatagramSize ?? 1200;
        for (const message of encodeDatagramAckBatches(acks, max)) {
          datagrams.write(message).catch(ignore);
        }
      },
      notice: null,
      close() {
        isOpen = false;
        try {
          session.close({ closeCode: WEBTRANSPORT_CLOSE.Normal, reason: "" });
        } catch {
          // Already closed.
        }
      },
    };
    began(l);
    // Checks for a stall also while nothing arrives at all.
    const watchdog = setInterval(() => {
      if (link === l) stalled(l);
    }, 1000);

    void (async () => {
      const reader = stream.readable.getReader();
      const frames = new StreamFrames();
      for (;;) {
        let chunk: Awaited<ReturnType<typeof reader.read>>;
        try {
          chunk = await reader.read();
        } catch {
          return; // The session closed.
        }
        if (chunk.done || link !== l) return;
        let complete: ArrayBuffer[];
        try {
          complete = frames.push(chunk.value);
        } catch (e) {
          dispatchError(e instanceof Error ? e : new Error(String(e)));
          l.close();
          return;
        }
        for (const frame of complete) onFrame(l, frame, now());
      }
    })();
    void (async () => {
      const reader = session.datagrams.readable.getReader();
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done || link !== l) return;
          onDatagram(l, value, now());
        }
      } catch {
        // The session closed.
      }
    })();
    // The server's closing frame names the code and reason; the browser's
    // `closed` does not get them from a server close.
    const closedWith = (info: { closeCode?: number; reason?: string } | null) => {
      isOpen = false;
      clearInterval(watchdog);
      const code = l.notice?.code ?? info?.closeCode;
      const reason = l.notice?.reason ?? info?.reason ?? "";
      const refused = code === WEBTRANSPORT_CLOSE.Policy && reason.startsWith("unauthorized");
      ended(l, refused ? reason : null);
    };
    session.closed.then(
      (info) => closedWith(info ?? null),
      () => closedWith(null),
    );
  };

  const openWebSocket = (ticket: string | undefined) => {
    const url = buildWsUrl();
    let ws: WebSocket;
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
    const l: Link = {
      kind: "websocket",
      get open() {
        return ws.readyState === WebSocket.OPEN;
      },
      send(input) {
        ws.send(input);
      },
      ack() {},
      notice: null,
      close() {
        ws.close();
      },
    };
    // Frames and the close belong to this socket once it is the link; a
    // socket that never opened reports its close all the same.
    link = l;

    ws.onopen = () => began(l);

    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) onFrame(l, event.data, now());
    };

    ws.onerror = () => {
      dispatchError(new Error(`WebSocket error connecting to shard ${currentShard}`));
    };

    ws.onclose = (event?: { code?: number; reason?: string }) => {
      const refused =
        event?.code === POLICY_CLOSE && (event.reason ?? "").startsWith("unauthorized");
      ended(l, refused ? (event?.reason ?? "") : null);
    };
  };

  connect();

  return {
    get connected() {
      return connected;
    },
    get transport() {
      return link?.kind ?? lastKind;
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
      if (!link || !connected || !link.open) {
        dispatchError(new Error("Cannot send: shard connection is not open"));
        return 0;
      }
      clientSeq += 1;
      link.send(encodeShardInput(codec, input, clientSeq));
      sentAt.set(clientSeq, now());
      if (sentAt.size > MAX_TIMED) sentAt.delete(sentAt.keys().next().value as number);
      return clientSeq;
    },
    close() {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      attempts += 1;
      opening?.abort();
      link?.close();
    },
  };
}
