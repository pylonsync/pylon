/**
 * A shard connection with no framework: one WebSocket or WebTransport
 * session, reconnected with backoff, that decodes frames, applies
 * replication frames to an `EntityTable`, and sends inputs. `useShard` in
 * `@pylonsync/react` and `connectShardGame` build on it.
 */

import { ShardClock } from "./clock";
import { EntityTable, readDatagramHeader, type ReplicationSummary } from "./replication";
import {
  SHARD_PROTOCOL_VERSION,
  ShardFrameKind,
  WEBTRANSPORT_CLOSE,
  StreamFrames,
  decodeWebTransportInfo,
  type WebTransportInfo,
  encodeDatagramAcks,
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
  close(): void;
}

/** The parts of the browser's `WebTransport` the client uses. */
interface WebTransportSession {
  readonly ready: Promise<void>;
  readonly closed: Promise<{ closeCode?: number; reason?: string } | undefined>;
  readonly datagrams: {
    readonly readable: ReadableStream<Uint8Array>;
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
    // Inputs sent on the old connection are never acknowledged on this
    // one (acks restart with the connection).
    sentAt.clear();
    for (const h of openHandlers) h();
  };

  /** A per-tick frame or datagram arrived for `tick`, acking `ack`. */
  const observe = (tick: number, ack: number, at: number) => {
    // The first arrival for a tick times the clock best; a late stream
    // frame after a newer datagram would move it back.
    if (tick > linkTick) {
      clock.observe(tick, at);
      linkTick = tick;
    }
    if (ack > linkAck) linkAck = ack;
    lastTick = linkTick;
    lastAck = linkAck;
    timeAcks(linkAck, at);
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
      // The new shard answered: a ticket function gives the next tickets.
      if (typeof options.ticket === "function") transferTicket = null;
      if (frame.kind === ShardFrameKind.Replication || frame.kind === ShardFrameKind.Snapshot) {
        // Only the per-tick frames: a rejection can go out before its
        // tick's frame is built, and would make the clock run early.
        observe(frame.tick, frame.ack, at);
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
        // A frame applied: the connection works, so the next reconnect
        // starts from the short delay again. (Resetting on open would
        // retry a frame that always fails every 500 ms forever.)
        backoff = options.reconnectBackoffMs ?? 500;
        for (const h of replicationHandlers) h(entities, summary, lastTick, lastAck);
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

  /**
   * A WebTransport datagram: entity updates. Returns the ack to send, or
   * null. A datagram older than a tick already seen is dropped unacked,
   * as if lost (the server sends its changes again), so handlers and the
   * clock only see ticks in order.
   */
  const onDatagram = (from: Link, datagram: Uint8Array, at: number): Uint8Array | null => {
    if (from !== link) return null;
    try {
      const header = readDatagramHeader(datagram);
      if (header.tick < linkTick) return null;
      const s = entities.applyDatagram(datagram);
      observe(s.tick, s.ack, at);
      backoff = options.reconnectBackoffMs ?? 500;
      const summary: ReplicationSummary = { full: false, spawned: [], updated: s.updated, despawned: [] };
      for (const h of replicationHandlers) h(entities, summary, lastTick, lastAck);
      return encodeDatagramAcks([[s.frame, entities.streamTick]]);
    } catch (e) {
      // Not a datagram this client can read: start over with a baseline.
      dispatchError(e instanceof Error ? e : new Error(String(e)));
      entities.clear();
      from.close();
      return null;
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
    if (closed || attempt !== attempts) {
      wt.close();
      return;
    }

    const writer = stream.writable.getWriter();
    const datagrams = (wt.datagrams.createWritable?.() ?? wt.datagrams.writable)?.getWriter();
    if (!datagrams) {
      wt.close();
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
      close() {
        isOpen = false;
        try {
          wt.close({ closeCode: WEBTRANSPORT_CLOSE.Normal, reason: "" });
        } catch {
          // Already closed.
        }
      },
    };
    began(l);

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
      const reader = wt.datagrams.readable.getReader();
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done || link !== l) return;
          const ack = onDatagram(l, value, now());
          if (ack) datagrams.write(ack).catch(ignore);
        }
      } catch {
        // The session closed.
      }
    })();
    wt.closed.then(
      (info) => {
        isOpen = false;
        const refused =
          info?.closeCode === WEBTRANSPORT_CLOSE.Policy && (info.reason ?? "").startsWith("unauthorized");
        ended(l, refused ? (info?.reason ?? "") : null);
      },
      () => {
        isOpen = false;
        ended(l, null);
      },
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
