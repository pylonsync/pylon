// Common interface every real-time transport implements + the host
// contract the engine offers each transport.
//
// Why this exists: WebSocket, SSE, and polling each used to live as
// large parallel methods on the engine, sharing state through engine
// fields (`this.ws`, `this.pingTimer`, `this.reconnectAttempts`,
// `this.pollTimer`). That coupling made the three modes hard to
// test, hard to swap, and a graveyard for state bugs (timer leaks on
// promotion, stale-socket sends after reconnect, ping racing close).
// The interface here gives each one a single owner of its own state
// + a tight contract with the engine for dispatch and lifecycle.

import type { ChangeEvent, SyncConnectionStatus } from "../types";

/** Real-time transport implementations all conform to this shape. The
 *  engine holds exactly one. The interface intentionally hides the
 *  underlying mechanism (WebSocket vs EventSource vs setInterval) so
 *  the engine can be told "go connect" without caring which one. */
export interface Transport {
  /** Connect / start the transport. Idempotent — calling twice is a
   *  no-op for an already-running transport. */
  start(): void;
  /** Disconnect / stop the transport, clearing every timer and
   *  releasing the socket. Idempotent. After stop() the transport is
   *  re-startable. */
  stop(): void;
  /** Send a JSON message over the transport. No-op for non-bidirectional
   *  transports (SSE, polling) — the engine still sends subscribe
   *  frames + presence + topic + ping through this entry point, and
   *  those frames simply never reach the wire when the transport
   *  doesn't support uplink. */
  send(msg: unknown): void;
  /** True when the underlying connection is currently open for sending.
   *  Used by the engine's `connected` getter and by paths that need to
   *  short-circuit when no socket exists (e.g., avoid throwing on a
   *  send-without-WS). */
  isOpen(): boolean;
  /** Bump the reconnect backoff counter by N attempts. Called by the
   *  engine when it observes a hint that the NEXT reconnect should
   *  wait longer — typically a 429 on pull, which would otherwise
   *  drive a tight 429 / reconnect / 429 loop. Transports without a
   *  backoff (polling) implement this as a no-op. */
  bumpReconnect(by?: number): void;
}

/** Engine-side surface the transport calls into. Transport is the
 *  thing that knows about sockets and timers; the host (engine) is the
 *  thing that knows what to DO with inbound frames and lifecycle
 *  events. Keeping this small + explicit makes the boundary clear:
 *  if a transport needs a new engine capability it gets added here,
 *  not via reaching back into the engine instance. */
export interface TransportHost {
  /** Base URL of the Pylon HTTP API (e.g. `https://api.example.com`).
   *  Transports derive their own URLs from this (ws upgrade path, SSE
   *  endpoint, poll path). */
  readonly baseUrl: string;
  /** Optional explicit WS URL — overrides the derived one. */
  readonly wsUrl?: string;
  /** WS keepalive interval. Default is implementation-defined; the
   *  engine accepts `pingIntervalMs` from config to tune broadcast
   *  latency on single-threaded server fallbacks. */
  readonly pingIntervalMs?: number;
  /** Base delay for reconnect backoff (full-jitter). */
  readonly reconnectDelayMs?: number;
  /** Polling cadence (polling transport only). */
  readonly pollIntervalMs?: number;

  /** Current bearer token, or undefined when anonymous. The transport
   *  must call this fresh on each connect / reconnect so token rotation
   *  is picked up automatically. */
  getToken(): string | undefined;

  /** Sync-relay mode (engine sets this only when `config.relay` is
   *  on): resolve the relay socket target freshly for THIS connect
   *  attempt — the ws `url` (with the `since` cursor) plus the signed
   *  auth `blob`. The transport sends the blob in the `bearer.<blob>`
   *  subprotocol, exactly like the machine token, so the credential
   *  stays out of the URL (and out of proxy logs). `null` = the token
   *  fetch failed; the transport backs off and retries like any failed
   *  connect. When present, this replaces `wsUrl`/derivation and the
   *  machine bearer token. */
  getRelayTarget?(): Promise<{ url: string; blob: string } | null>;

  /** Is this tab the multi-tab leader. Followers don't open their own
   *  transport — they mirror the leader's broadcasts. Transports check
   *  this defensively at start() to avoid wasting socket budget. */
  isLeader(): boolean;
  /** Is the engine currently running. Set false on stop(); transports
   *  bail out of reconnect timers when this flips. */
  isRunning(): boolean;

  /** Inbound: a typed change event from the server's change log.
   *  Engine routes through the apply queue so order is preserved. */
  onChangeEvent(ev: ChangeEvent): void;
  /** Inbound: a typed JSON envelope (not a ChangeEvent). The transport
   *  doesn't try to interpret the envelope — engine has the dispatch
   *  table (session-changed, reactive-result, row-revoked, presence,
   *  pong, etc). */
  onJsonMessage(msg: Record<string, unknown>): void;
  /** Inbound: a binary frame. CRDT broadcast layer ships these for
   *  every Loro-mode write; the engine routes via its binaryHandlers
   *  + multi-tab forwarders. */
  onBinaryFrame(bytes: Uint8Array): void;

  /** Lifecycle: the transport just connected. Engine uses this to
   *  replay active subscriptions, fire a catch-up pull + reconcile,
   *  and flip the public connection status. */
  onConnected(): void;
  /** Lifecycle: the transport just disconnected. Engine flips status
   *  to `reconnecting` while the transport's own backoff timer is
   *  pending; or to `offline` when `isRunning()` is false. */
  onDisconnected(): void;
  /** Flip the engine's public `connectionStatus`. Transports drive
   *  this so the UI sees `connecting` / `connected` / `reconnecting`
   *  / `offline` at the right moments. */
  setStatus(s: SyncConnectionStatus): void;

  /** Polling transport only: one iteration of the push→pull cycle.
   *  Implementations of polling do nothing else; they just call this
   *  on a timer. */
  performPollTick(): Promise<void>;
  /** Reconnect-aware transports (WS, SSE) need to do a catch-up pull
   *  before the next connect attempt so they don't open a fresh socket
   *  and immediately discover their cursor is stale. The engine owns
   *  the pull queue + reconcile path; the transport invokes this. */
  performReconnectPull(): Promise<void>;
}
