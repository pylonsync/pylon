// ---------------------------------------------------------------------------
// pylon sync client
//
// JSON live-query sync with optimistic mutations and an offline write queue.
// CRDT-mode rows travel through the same WebSocket as binary Loro frames,
// but this package intentionally keeps binary payloads opaque and routes
// them to consumers such as @pylonsync/loro. See docs/SYNC.md for the full
// projection + convergence model.
// ---------------------------------------------------------------------------

import {
  pylonFetch,
  PylonHttpError,
  type TransportConfig,
} from "./transport";
import { LocalStore } from "./local-store";
import { MutationQueue, type PendingMutation } from "./mutation-queue";
import { MultiTabOrchestrator } from "./multi-tab-orchestrator";
import { OpQueue } from "./op-queue";
import {
  RoomSubscriptions,
  type RoomError,
  type RoomMember,
  type RoomSubscriber,
} from "./room-subscriptions";
import { ServerSubscriptions } from "./server-subscriptions";
import { SessionResolver } from "./session-resolver";
import { SubscriptionCoordinator } from "./subscription-coordinator";
import {
  createTransport,
  type Transport,
  type TransportHost,
  type TransportKind,
} from "./transports";
import { generateClientId, generateId } from "./ids";
export {
  RoomSubscriptions,
  type RoomError,
  type RoomErrorCode,
  type RoomMember,
  type RoomMessage,
  type RoomMessageSubscriber,
  type RoomSubscriber,
} from "./room-subscriptions";
export { IndexedDBPersistence, persistChange } from "./persistence";
export {
  buildRequest,
  pylonFetch,
  pylonFetchRaw,
  PylonHttpError,
  resolveBaseUrl,
} from "./transport";
export type { PylonRequestInit, TransportConfig } from "./transport";
export { LocalStore } from "./local-store";
export {
  MutationQueue,
  type MutationQueuePersistence,
  type PendingMutation,
} from "./mutation-queue";
export { generateId } from "./ids";
export type {
  ChangeEvent,
  ClientChange,
  PullResponse,
  PushOpResult,
  PushResponse,
  ReactiveMessage,
  ReactiveSpec,
  ResolvedSession,
  Row,
  SyncConnectionStatus,
  SyncCursor,
  TransportType,
} from "./types";
export {
  defaultStorage,
  createWriteThroughStorage,
  type Storage,
} from "./storage";

import { defaultStorage } from "./storage";

// Type-only imports for the SyncEngine implementation that follows.
// Public exports of these types live at the top of this file (re-
// exported from `./types`).
import type {
  ChangeEvent,
  ClientChange,
  PullResponse,
  PushOpResult,
  PushResponse,
  ReactiveMessage,
  ReactiveSpec,
  ResolvedSession,
  Row,
  SyncConnectionStatus,
  SyncCursor,
  TransportType,
} from "./types";


// ---------------------------------------------------------------------------
// Sync engine — coordinates pull, push, local store, mutation queue
// ---------------------------------------------------------------------------

export interface SyncEngineConfig {
  baseUrl: string;
  /** Transport type. Default: "websocket". Falls back to polling if connection fails. */
  transport?: TransportType;
  /** WebSocket URL. Default: derived from baseUrl (ws://). */
  wsUrl?: string;
  /** Poll interval in ms (only used when transport is "poll"). Default 1000. */
  pollInterval?: number;
  /** Reconnect delay in ms. Default 1000. */
  reconnectDelay?: number;
  /** Auth token for requests. */
  token?: string;
  /** Enable IndexedDB persistence. Data survives page refresh. Default: true in browser. */
  persist?: boolean;
  /** App name for IndexedDB database naming. Default: "default". */
  appName?: string;
  /**
   * Sync key-value adapter for hot-path state (auth token, client_id).
   * Default: localStorage on the web, in-memory no-op elsewhere. Non-browser
   * hosts (RN, Tauri, Workers) inject an adapter to persist these values.
   */
  storage?: import("./storage").Storage;
  /**
   * Debounce window (ms) between `reconcile()` calls. Reconcile triggers
   * fire on connect, WS reconnect, and visibility-change; the debounce
   * prevents the back-to-back triggers from re-fetching every entity
   * twice within seconds. Default 2000ms.
   */
  reconcileMinIntervalMs?: number;
  /**
   * Opt out of the automatic visibility-change reconcile. The reconcile
   * pass runs on connect/reconnect regardless; this only disables the
   * tab-refocus trigger. Default: enabled (reconcile fires when the tab
   * becomes visible).
   */
  reconcileOnVisibility?: boolean;
  /**
   * Client → server keepalive interval (ms). The Pylon dev server's
   * dedicated :port+1 WS listener uses a dual-thread design where the
   * writer thread wakes on every broadcast — pings are pure liveness,
   * 25_000 (25s) is the right default. The HTTP-multiplexed
   * `/api/sync/ws` fallback path uses a single-thread design where
   * the reader's mutex unlocks only between reads; on THAT path,
   * broadcast latency is bounded by this interval, so set it lower
   * (e.g. 200) to trade traffic for delivery latency. Most production
   * deployments should leave this at the default and configure their
   * edge proxy to forward to the dual-thread listener instead.
   */
  pingIntervalMs?: number;
  /**
   * Multi-tab coordination via BroadcastChannel. When multiple tabs of
   * the same origin run the engine, one is elected leader and owns
   * the WebSocket, pull/push/reconcile, and IndexedDB writes;
   * follower tabs mirror state via cross-tab broadcasts. Default
   * `true` in browsers. Set `false` to force every tab to behave as
   * its own leader (the pre-multi-tab semantics).
   */
  multiTab?: boolean;
}

/**
 * Generate a stable client_id. Prefers a persisted id from `storage`
 * (so a reload keeps the same identifier) and falls back to a fresh UUID.
 */
/**
 * Generate a Pylon-shaped row id (40-char lowercase hex).
 *
 * Mirrors the runtime's `generate_id` shape: 32 hex of milliseconds
 * since epoch (extended to nanos so it lex-sorts alongside server-
 * generated ids) + 8 hex of a per-tab counter. Lex-sortable, monotonic
 * within a tab, statistically unique across tabs (the timestamp
 * disambiguates almost every cross-tab collision; the counter handles
 * the rest within a single tick).
 *
 * Used by `useMutation({ optimistic })` to mint client-side ids that
 * the runtime will accept verbatim — so the optimistic ghost and the
 * canonical row share the same `row_id` and the WS broadcast is an
 * idempotent merge instead of a delete-then-replace flash.
 *
 * Apps can call this directly when they need a stable id earlier than
 * the mutation (e.g. to reference the row from another optimistic
 * insert in the same gesture).
 */

export class SyncEngine {
  private config: SyncEngineConfig;
  private cursor: SyncCursor = { last_seq: 0 };
  private running = false;
  /** Real-time transport — owns its own socket / timers / backoff.
   *  The engine just calls start/stop/send and consumes inbound events
   *  via the TransportHost callbacks (set up below in `transportHost`).
   *  Constructed in start() because followers don't open a transport
   *  at all and SSR-only consumers never reach start(). */
  private transport: Transport | null = null;
  private _connectionStatus: SyncConnectionStatus = "offline";
  private persistence: import("./persistence").IndexedDBPersistence | null = null;

  /**
   * Flips true once `start()` has either:
   *   - drained IndexedDB into the in-memory store (data path), OR
   *   - decided the engine has no persistence layer (test / SSR / explicit opt-out).
   *
   * `useQuery`'s `loading` flag consumes this so apps don't render a
   * "Loading…" flash on every page refresh when the disk replica
   * already has the data. Without it, even returning visits show the
   * spinner for the 50–200ms gap between component mount and IndexedDB
   * `loadAllEntities()` resolving — visually identical to a true
   * cold-start fetch.
   */
  private _hydrated = false;
  isHydrated(): boolean {
    return this._hydrated;
  }

  /**
   * True once the engine has a *server-confirmed* initial view: the first
   * pull (snapshot) settled, OR the IndexedDB cache already held rows, OR a
   * fallback deadline elapsed so we never pin forever. This is what
   * `useQuery`'s `loading` waits on — NOT `isHydrated()`. The distinction
   * matters: `isHydrated()` flips true the instant the local cache loads,
   * which on a cold/empty cache (first visit, or right after an org switch
   * wipes the replica) is immediate and EMPTY — so a `loading` gated on it
   * drops to false while the projects/rows are still en route from the
   * server, and the UI flashes its empty state for the ~seconds until the
   * pull lands. Gating on this signal keeps `loading` true through that
   * window so callers can show a skeleton instead.
   */
  private _initialSyncSettled = false;
  isInitialSyncSettled(): boolean {
    return this._initialSyncSettled;
  }

  private _initialSyncFallback: ReturnType<typeof setTimeout> | null = null;

  /** Flip `_initialSyncSettled` true (idempotent) + notify so `useQuery`
   *  re-reads and drops its loading state. */
  private markInitialSyncSettled(): void {
    if (this._initialSyncFallback !== null) {
      clearTimeout(this._initialSyncFallback);
      this._initialSyncFallback = null;
    }
    if (this._initialSyncSettled) return;
    this._initialSyncSettled = true;
    this.store.notify();
  }

  /** Safety net so `loading` never pins: settle after a deadline even if no
   *  pull lands (offline, or a multi-tab follower of an empty entity that
   *  never receives a broadcast). The real pull settles it far sooner in the
   *  normal case. Re-armable — a replica wipe (org switch / token flip) resets
   *  the signal and re-arms this. */
  private armInitialSyncFallback(): void {
    if (this._initialSyncFallback !== null) clearTimeout(this._initialSyncFallback);
    this._initialSyncFallback = setTimeout(() => {
      this._initialSyncFallback = null;
      this.markInitialSyncSettled();
    }, 12_000);
  }

  /**
   * True when the engine drained at least one row OR a saved cursor
   * out of IndexedDB during `start()`. Distinguishes a returning user
   * (cached replica may contain rows the server has since deleted) from
   * a true first-time user (cache empty, pull-from-0 IS canonical
   * truth).
   *
   * Used by the WS `onConnected` fast-path: `lastPullStartedFromZero`
   * only fires the reconcile-skip when this flag is ALSO false. A
   * returning user whose IDB cursor somehow rolled back to 0 (rare:
   * partial wipe, corrupt write) must still get the reconcile pass —
   * otherwise rows deleted on the server while the tab was closed
   * survive forever.
   *
   * Read-only after start() observes the IDB load.
   */
  private _hadCachedReplica = false;

  /**
   * Which identity (resolved `userId`, or `null` when logged out) the
   * currently-hydrated replica belongs to. Loaded from disk on cold start
   * and re-tagged after every (re)sync. `undefined` means "unknown" — a
   * pre-tag replica or a fresh engine; the guard must NOT treat that as a
   * mismatch. Used to wipe the replica when a different user signs in on
   * the same browser across a page reload — the leak `observeToken` can't
   * see, because a fresh engine has no prior token to compare against.
   */
  private _replicaIdentity: string | null | undefined = undefined;

  /**
   * Sticky flag: a persisted row/cursor write degraded (IDB quota /
   * abort), so the on-disk replica is known to be behind the in-memory
   * cursor. Once set, `enqueueApply` STOPS advancing the persisted
   * cursor — persisting a cursor ahead of the durable rows would make
   * the next cold start skip them forever (cursor-ahead-of-replica). The
   * in-memory replica stays authoritative for the live session; on
   * restart the lagging on-disk cursor simply re-pulls the gap. Resets to
   * false only on `resetReplicaInner` (full wipe + resync, disk is clean
   * again). A storage-pressured tab thus degrades to "re-pull on restart"
   * — like a memory-only client — instead of silently losing rows.
   */
  private persistDegraded = false;

  readonly store: LocalStore;
  readonly mutations: MutationQueue;

  /**
   * Stable per-client identifier. Minted on first construction, not
   * necessarily persisted (depends on what the host provides).
   * Included on every PushRequest so the server can correlate retries and
   * track per-client diagnostics. Not auth — do not trust this to identify
   * a user.
   */
  readonly clientId: string;

  /** Presence state for this client. */
  private presenceData: Record<string, unknown> = {};

  /**
   * Token observed on the last pull. When the token changes (anonymous →
   * signed in, or user A → user B), the set of rows the server will expose
   * changes — so the cursor from the previous identity is meaningless.
   * Compared on every pull; a mismatch triggers an automatic resync.
   *
   * Owns the resolved session, the last-seen token, the last-seen
   * tenant, and the null→X / X→Y / token-flip verdicts that used to
   * be inlined across pull / refresh / reconcile. The engine acts on
   * the verdicts (reset, pull, notify); the resolver decides nothing
   * on its own. See session-resolver.ts.
   *
   * Exposed (read-only) so tests and plugins can inspect or simulate
   * identity transitions without re-implementing the comparison.
   */
  readonly session: SessionResolver = new SessionResolver();

  /**
   * Multi-tab orchestrator — owns the cross-tab protocol: broker
   * lifecycle, election, inbound message dispatch, outbound broadcasts.
   * Engine receives inbound events via hooks (see `multiTabHooks()`).
   *
   * Constructed lazily in `start()` because SSR-only consumers that
   * never reach start() shouldn't pay for the broker. */
  private orchestrator: MultiTabOrchestrator | null = null;
  /** Mirror of `orchestrator.isLeader()` kept on the engine for the
   *  many `if (this.isMultiTabLeader)` gates throughout the codebase.
   *  Updated via the orchestrator's onInitialLeader / onLatePromote /
   *  onDemote hooks. Defaults to false so a tab joining an existing
   *  election stays a passive follower until the orchestrator
   *  explicitly promotes us — a `true` default would let a late
   *  joiner whose orchestrator never fires onPromote (because it was
   *  never leader) silently run as a leader. */
  private isMultiTabLeader = false;

  /**
   * Serialized apply queue. Every change-event apply — from WS onmessage,
   * pull(), or session-changed catchup — chains onto this promise so
   * applies execute in arrival order. Without this, two WS messages or
   * two concurrent pull()s race: seq 3's persistence can land before
   * seq 2's, leaving the row at the older value AND the cursor briefly
   * regressing if writes complete out of order. The queue also gates
   * the cursor advance so `last_seq` only moves forward.
   */
  private applyQueue: Promise<void> = Promise.resolve();

  /**
   * Live-event hold buffer, active ONLY while a from-zero snapshot pull is in
   * flight. A snapshot is full state as-of `snapshot_seq` S; its rows arrive
   * tagged `seq = S`. If a live WS frame (or a tab broadcast) at `seq = S+k`
   * applies FIRST — during the snapshot's (possibly multi-page) HTTP fetch — it
   * advances the cursor past S, and then EVERY snapshot row (seq ≤ S) is
   * dropped by the monotonic filter in enqueueApply, leaving a near-empty
   * replica with the cursor persisted ahead (no 410, no heal until a reconcile
   * happens to fire). The store has no per-row seq guard, so we can't just
   * apply the snapshot unconditionally — an older snapshot row would clobber a
   * newer live update. So we instead ORDER them: hold live/broadcast applies
   * here while snapshotting, then replay them (seq-filtered) AFTER the snapshot
   * lands. null = not snapshotting → normal apply.
   */
  private snapshotHold: ChangeEvent[] | null = null;

  /**
   * Serialized channel for outbound network ops (pull, push, reconcile,
   * refresh, resetReplica). Replaces the per-op `inFlightX` mutexes +
   * the fire-and-forget `void refreshResolvedSession()` calls that used
   * to race against in-flight pulls and reconciles. Apply stays
   * separate (see `applyQueue` above) so WS events don't block on a
   * pull's HTTP round-trip.
   */
  private opQueue: OpQueue = new OpQueue();

  /**
   * Serialized chain for session-transition application. Multiple
   * concurrent triggers (`refreshResolvedSession` from app code +
   * a `session-changed` envelope landing over WS + a multi-tab
   * `session` broadcast) all enqueue here. Without it, two
   * inspect-then-commit pairs interleave on the microtask queue
   * and the older session can commit AFTER the newer, leaving the
   * engine pinned to a stale tenant.
   */
  private sessionChain: Promise<void> = Promise.resolve();

  /**
   * Registered consumers for binary WebSocket frames. SyncEngine itself
   * doesn't decode binary — it just owns the WS connection and routes
   * frames to whoever signed up via [`onBinaryFrame`]. The first
   * consumer is `@pylonsync/loro` for CRDT snapshots / updates;
   * future binary use cases (file streaming, etc.) register the same
   * way so this layer stays use-case-agnostic.
   *
   * Set rather than Array so a hot-reload re-registration of the same
   * handler doesn't double-invoke. Caller-provided handler identity
   * is the dedup key.
   */
  private binaryHandlers: Set<(bytes: Uint8Array) => void> = new Set();

  /**
   * Server-side ephemeral subscriptions (CRDT row subs, reactive query
   * subs, future kinds). Owns the WS replay bookkeeping — each kind
   * registers the message that re-creates its server-side state, and
   * `ws.onopen` replays the bundle on reconnect. Kind-specific concerns
   * (CRDT refcount, reactive handler routing) stay below as their own
   * maps. See server-subscriptions.ts.
   */
  private serverSubs!: ServerSubscriptions;

  /** Coordinator for every "this tab wants live updates" subscription —
   *  CRDT row subs + reactive query subs, leader bookkeeping + follower
   *  forwarding. The engine delegates `subscribeCrdt` / `unsubscribeCrdt`
   *  / `subscribeReactive` / `unsubscribeReactive` to it, and routes
   *  inbound multi-tab `sub-register` / `sub-unregister` / `reactive-msg`
   *  envelopes through it. Constructed lazily in start() because
   *  serverSubs isn't built until then. */
  private subscriptions!: SubscriptionCoordinator;

  /** Room presence subscriptions. Replaces the per-component
   *  setInterval(GET /api/rooms/<room>, 5s) polling loop the `useRoom`
   *  hook used to run for every channel. New server protocol
   *  (v0.3.214+): the client sends `room-subscribe` / `room-unsubscribe`
   *  over the existing WS, and the server pushes `room-snapshot` /
   *  `room-update` whenever membership changes.
   *
   *  This engine field is leader-only (followers forward register /
   *  unregister calls over the multi-tab channel — the leader's
   *  registry is the single source of truth on the wire). Constructed
   *  lazily in start() so SSR-only callers don't pay for it. */
  private rooms!: RoomSubscriptions;

  /** Per-room set of follower tabIds that have forwarded a
   *  `room-sub-register`. Leader-only. Mirrors `crdtForwarders` in the
   *  SubscriptionCoordinator — the WS room sub stays alive until both
   *  the local refcount AND this set are empty. A separate map on the
   *  engine (instead of inside RoomSubscriptions) because the registry
   *  is leader-local and forwarder bookkeeping is multi-tab specific. */
  private roomForwarders: Map<string, Set<string>> = new Map();

  /**
   * Listeners notified when the server signals a per-subscriber row
   * revocation (`row-revoked` envelope). Used by `@pylonsync/loro`
   * to evict the LoroDoc registry entry for a row whose policy was
   * revoked mid-session. Plain Set so identity is the dedup key.
   */
  private rowEvictionListeners: Set<(entity: string, rowId: string) => void> =
    new Set();

  /**
   * Register a binary-frame handler. Returns an unsubscribe fn that
   * pulls the handler back out — call on hook unmount / module
   * teardown so handlers don't leak.
   *
   * Multiple handlers can register concurrently; each gets called for
   * every binary frame the WS receives. Handlers should be cheap and
   * non-throwing — exceptions are caught and logged but the message
   * is otherwise dropped for that handler.
   */
  onBinaryFrame(handler: (bytes: Uint8Array) => void): () => void {
    this.binaryHandlers.add(handler);
    return () => {
      this.binaryHandlers.delete(handler);
    };
  }

  /** Read the cached resolved session. Null user = anonymous. */
  resolvedSession(): ResolvedSession {
    return this.session.resolved();
  }

  /**
   * Coarse connection state (see `SyncConnectionStatus`). Updated as
   * the WS opens/closes and reconnect attempts run; subscribers re-
   * render via the same store notify channel as live queries, so
   * `useSyncStatus` is just a thin reader.
   */
  connectionStatus(): SyncConnectionStatus {
    return this._connectionStatus;
  }

  /**
   * Count of optimistic writes still queued locally (not yet acked by the
   * server) — i.e. the offline outbox depth. 0 when fully synced. Read by the
   * dev HUD's sync row; also handy for a "saving…" indicator.
   */
  pendingCount(): number {
    return this.mutations.pending().length;
  }

  /**
   * Mutate connection status + notify subscribers. Idempotent — same-
   * status calls are a no-op so the WS onopen → connected transition
   * doesn't spam re-renders during a stable connection.
   */
  private setConnectionStatus(next: SyncConnectionStatus): void {
    if (this._connectionStatus === next) return;
    this._connectionStatus = next;
    this.store.notify();
  }

  /** Sync key-value adapter for hot-path state (token, client_id). */
  readonly storage: import("./storage").Storage;

  constructor(config: SyncEngineConfig) {
    this.config = config;
    this.store = new LocalStore();
    this.mutations = new MutationQueue();
    this.storage = config.storage ?? defaultStorage();
    this.clientId = generateClientId(this.storage);
    // ServerSubscriptions defers sending until the WS is open (the
    // sendWs helper short-circuits otherwise) so registering before
    // start() is safe — the spec gets replayed on `ws.onopen`.
    this.serverSubs = new ServerSubscriptions((msg) => this.sendWs(msg));
    this.subscriptions = new SubscriptionCoordinator(this.serverSubs, {
      isLeader: () => this.isMultiTabLeader,
      broadcastToTabs: (payload) => this.broadcastToTabs(payload),
    });
    this.rooms = new RoomSubscriptions((msg) => {
      // Leader: send over the WS. Followers don't open a transport;
      // the leader-side WS is the only path to the server.
      if (!this.isMultiTabLeader) return false;
      if (this.transport?.isOpen()) {
        this.transport.send(msg);
        return true;
      }
      return false;
    });
    // When multi-tab coordination is explicitly disabled, this engine
    // is always its own sole leader — even before start(). Tests that
    // construct an engine and call reconcile()/pull() directly without
    // start() rely on this. The dynamic election paths (broker-driven
    // promote/demote) only matter when multiTab is enabled.
    if (this.config.multiTab === false) {
      this.isMultiTabLeader = true;
    }
    // Dev HUD hook: in dev (the server injects `__PYLON_DEV__` into the page),
    // publish a tiny read-only status probe the floating dev overlay polls for
    // the sync row (connection + offline-outbox depth). Gated on the dev marker
    // so it's never present in production. Defensive — never let it break init.
    try {
      const g = globalThis as Record<string, unknown>;
      if (typeof window !== "undefined" && g.__PYLON_DEV__) {
        g.__pylonDevSync = {
          status: () => this.connectionStatus(),
          pending: () => this.pendingCount(),
          rows: () => this.store.size(),
        };
      }
    } catch {
      // ignore — the HUD just won't show a sync row
    }
  }

  /**
   * Hydrate the local store with server-rendered data.
   * Call this before start() to avoid a redundant initial pull.
   * Typically used for SSR: server fetches data + cursor, passes to client.
   */
  hydrate(data: HydrationData): void {
    for (const [entity, rows] of Object.entries(data.entities)) {
      for (const row of rows) {
        const id = (row as Record<string, unknown>).id as string;
        if (id) {
          this.store.applyChange({
            seq: 0,
            entity,
            row_id: id,
            kind: "insert",
            data: row as Record<string, unknown>,
            timestamp: "",
          });
        }
      }
    }
    if (data.cursor) {
      this.cursor = data.cursor;
    }
  }

  /** Start the sync engine. Loads persisted data, pulls updates, then connects for real-time. */
  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;
    this.setConnectionStatus("connecting");
    warnIfMisconfigured(this.config.baseUrl);
    // Arm the loading-settle safety net before any async work, so a multi-tab
    // follower (which never pulls) or an offline start can't pin loading.
    this.armInitialSyncFallback();

    // Load persisted data if available.
    const shouldPersist = this.config.persist !== false && typeof indexedDB !== "undefined";
    if (shouldPersist) {
      try {
        const { IndexedDBPersistence } = await import("./persistence");
        this.persistence = new IndexedDBPersistence(this.config.appName);
        await this.persistence.open();

        // Warm-load entities + cursor in ONE readonly transaction so
        // the hydrated rows and the cursor we'll advance from are a
        // consistent snapshot. Separate reads could (in a multi-tab
        // race) interleave a mid-load save and read (rows@C, cursor@C+1)
        // — the pull would then skip seqs we never applied. The
        // post-load timing log surfaces cold-IDB pages so a regression
        // (50MB cache, slow disk) is observable.
        const idbLoadStart =
          typeof performance !== "undefined" ? performance.now() : Date.now();
        const { entities: cached, cursor: cachedCursor, hadCache } =
          await this.persistence.loadSnapshot();
        const idbLoadMs =
          (typeof performance !== "undefined" ? performance.now() : Date.now()) -
          idbLoadStart;
        if (idbLoadMs > 100) {
          console.warn(
            `[persistence] cold IDB load took ${idbLoadMs.toFixed(0)}ms (${
              Object.keys(cached).length
            } entities)`,
          );
        }
        // Record whether IDB had a prior session's state. The cold-load
        // fast-path in onConnected (skip post-pull reconcile when the
        // pull was a full snapshot from cursor=0) is only safe when
        // there was no cached replica to begin with — a returning user
        // whose pull-from-cursor misses an offline server-side delete
        // depends on that reconcile pass to catch the ghost row.
        this._hadCachedReplica = hadCache;
        // Which identity the hydrated rows belong to. Compared against the
        // resolved session in `guardReplicaIdentity` before the first pull
        // so a reload-time account switch drops the prior user's rows.
        this._replicaIdentity = await this.persistence.loadIdentity();
        let hydrated = false;
        for (const [entity, rows] of Object.entries(cached)) {
          for (const row of rows) {
            const id = (row as Record<string, unknown>).id as string;
            if (id) {
              this.store.applyChange({ seq: 0, entity, row_id: id, kind: "insert", data: row, timestamp: "" });
              hydrated = true;
            }
          }
        }
        // applyChange() doesn't notify — it's the low-level primitive.
        // Fire one notify after the hydration loop so useSyncExternalStore
        // subscribers re-read. Without this, if the subsequent pull returns
        // no changes (replica already at cursor), subscribers stay stuck on
        // their initial empty snapshot until the first WS event arrives.
        this._hydrated = true;
        if (hydrated) {
          this.store.notify();
          // Cache already held rows: we have a usable view now, so settle the
          // initial-sync signal immediately — a returning user sees data on
          // frame one; the pull below just refreshes it. (An EMPTY cache does
          // NOT settle here — loading stays true until the pull confirms,
          // which is the whole point: no empty-state flash before first sync.)
          this.markInitialSyncSettled();
        } else {
          this.store.notify();
        }

        // Apply the cached cursor BEFORE pull so the first pull is a
        // delta against where we left off, not a full re-snapshot.
        // Already part of the single loadAll() tx above — assigning
        // here can't race a concurrent save because pull/push haven't
        // started yet (initMultiTab is still ahead).
        if (cachedCursor) {
          this.cursor = cachedCursor;
        }

        // Auto-save changes to IndexedDB. Returns a Promise<boolean>
        // (true = durable) so the async apply path (applyChangesAsync)
        // can both await the write before the cursor advances AND hold
        // the persisted cursor back when a write degraded — the fix for
        // "cursor ahead of replica" on crash AND on quota/abort.
        const persistence = this.persistence;
        this.store._persistFn = async (change: ChangeEvent) => {
          const { persistChange } = await import("./persistence");
          if (!persistence) return true;
          return persistChange(persistence, change);
        };

        // Hydrate the mutation queue from disk. Any offline writes
        // queued before the tab was closed come back as pending here.
        //
        // Invariant: hydrated offline mutations reach the server
        // before reconcile inspects local state. Test:
        // `hydrated_offline_mutations_survive_startup_reconcile`.
        // Without an explicit push here, WS-only mode (no polling)
        // would let pull+reconcile sweep the optimistic ghosts before
        // push() ever fires.
        try {
          const { IndexedDBMutationPersistence } = await import("./persistence");
          const mqPersistence = new IndexedDBMutationPersistence(persistence);
          this.mutations.attachPersistence(mqPersistence);
          await this.mutations.hydrate();
          // The hydrated offline writes are drained in the leader path
          // below (after `initMultiTab` settles), NOT here. We're still
          // pre-election at this point, so `isMultiTabLeader` is false
          // and a push() now would hit the follower branch — broadcasting
          // the batch to a not-yet-constructed orchestrator (a silent
          // no-op) and stranding every offline write until an unrelated
          // mutation happened to fire push() again. The drain moved to
          // the leader path so it runs once we actually own the network.
          // Test: `hydrated offline writes are pushed once leader-elected`.
        } catch {
          // Queue persistence optional — memory-only still works.
        }
      } catch {
        // IndexedDB not available — continue without persistence.
      }
    }
    // Always flip `_hydrated` true by this point — even when persist
    // was off or IndexedDB threw. useQuery's loading semantics depend
    // on it; leaving false would pin every app with persist:false
    // into a permanent "Loading…" state.
    this._hydrated = true;

    // Multi-tab coordination: elect a leader before deciding whether
    // to open the WS / pull / poll. Followers stay passive and mirror
    // applied changes broadcast by the leader. The election settles
    // in ~250ms; if the broker is unavailable (no BroadcastChannel)
    // every tab is implicitly its own leader.
    //
    // Bootstrap parallelization: the election (~250ms) and
    // /api/auth/me (~60ms) are independent — kick both off, then
    // await election first. If we lose the election we discard the
    // session result and let the leader broadcast its session over
    // the multi-tab channel; the "leader-only network writes"
    // invariant is preserved because no peers have observed our
    // pending /api/auth/me request and no apply has happened yet.
    const electionPromise = this.initMultiTab();
    const sessionPromise = this.fetchSessionBootstrap().catch(() => null);
    await electionPromise;

    if (!this.isMultiTabLeader) {
      // Follower path: rely on the leader's broadcasts for session +
      // applied changes. Nothing else to do here — the broker is
      // wired to forward inbound messages into the engine. The
      // sessionPromise we kicked off above resolves into the void;
      // the leader's broadcast will deliver the authoritative view.
      // Swallow any pending error so it doesn't surface as an
      // unhandled rejection.
      void sessionPromise.then(
        () => {},
        () => {},
      );
      return;
    }

    // Leader path. If any subscribeCrdt / subscribeReactive call came
    // in before start() (or before initMultiTab settled), it took the
    // default-follower branch and tried to broadcast to a not-yet-
    // running broker. Now that we know we ARE the leader, populate
    // serverSubs from local interest so the WS subscribe frames go
    // out on the next connect. Idempotent w.r.t. subscribe calls that
    // happen later through the normal leader branch.
    this.seedServerSubsFromLocalInterest();

    // Seed the server-resolved session before the first pull so
    // `useSession` subscribers see the right tenant from frame one,
    // and the resolver's lastSeenTenant is populated before any
    // subsequent flip can race with it. We pre-fired the HTTP fetch
    // above (in parallel with election); apply its result now.
    // Falls through to a normal refresh on network/parse error so
    // we don't get stuck without a session.
    const bootstrapSession = await sessionPromise;
    if (bootstrapSession !== null) {
      await this.applySessionTransition(bootstrapSession, /* broadcast */ true);
    } else {
      await this.refreshResolvedSession();
    }

    // Identity-tag guard. If the hydrated replica belongs to a DIFFERENT
    // user than the one we just resolved — a shared browser / account
    // switch across a page reload — wipe it before the pull below layers
    // the new identity's deltas on top of the old user's rows. The live
    // `observeToken` reset can't catch this: a fresh engine's
    // `lastSeenToken` starts undefined, so its first observation reports
    // no change and it would otherwise adopt the prior user's replica.
    await this.guardReplicaIdentity();

    // Drain hydrated offline writes now that we ARE the leader. The
    // startup hydrate (in the persist block above) ran pre-election,
    // when a push() would have taken the follower branch and broadcast
    // into a not-yet-running orchestrator — a no-op that stranded the
    // writes. We own the network now, so the follower gate in pushInner
    // passes and the batch actually reaches /api/sync/push. Fire-and-
    // forget (op_ids dedupe against the broadcasts) and ahead of pull()
    // so the server has the writes before the cold-load snapshot lands —
    // the snapshot then returns them as canonical instead of leaving the
    // reconcile backstop to recover the optimistic ghosts.
    if (this.mutations.pending().length > 0) {
      void this.push();
    }

    // Pull from server, then connect real-time transport. `pull()` settles the
    // initial-sync signal on completion (leader path), so useQuery's loading
    // drops here on the normal fast path and the start() fallback is cancelled.
    await this.pull();

    // Save cursor after pull. Fire-and-forget on bootstrap — the
    // enqueueApply path already persists per-batch as pull lands rows,
    // so this final save is belt-and-braces. Awaiting it adds 5-30ms
    // of IDB tail latency to the critical path before transport.start
    // runs; the apply path's idempotent op_id-keyed merge handles the
    // worst case (one re-applied batch on next cold pull if the tab
    // crashes between this line and the saveCursor task completing).
    if (this.persistence && !this.persistDegraded) {
      void this.persistence.saveCursor(this.cursor);
    }

    // Tag the freshly-synced replica with the identity that owns it, so the
    // next cold start can detect an account switch (see guardReplicaIdentity).
    this.persistReplicaIdentity();

    // First-load reconciliation pass — closes the "phantom row" gap when
    // the local IndexedDB has rows the server doesn't (deletes made by
    // another surface while this tab was closed, or events that fell
    // off the in-memory ChangeLog before this tab's cursor caught up).
    //
    // Deliberately NOT fired here anymore. Apps that select-org from
    // a bootstrap effect race against this reconcile pass: the engine
    // resolves /api/auth/me before selectOrg lands, tenant=null, the
    // entity fetch returns 0 rows, every cached row gets tombstoned,
    // and the user sees a "rows render then flash away" gap until
    // selectOrg fires the session-changed envelope and the engine
    // re-pulls. The visibility-change + WS-reconnect reconcile triggers
    // below STILL run, so deletes made while the tab was closed
    // converge on the next focus / reconnect — just not in the narrow
    // window where the session might still be unresolved. Net effect:
    // identical safety, no flash.
    //
    // The session-flip guard in reconcileInner is the second line of
    // defense; this is the first. Belt + braces.

    // Wire the visibility-change reconcile so a tab that returns from
    // the background (laptop wakes, tab unhidden) catches up against
    // server truth without waiting for a WS event. Closes the "Stripe
    // webhook on a sibling Fly machine" / "missed WS event" gap.
    this.attachVisibilityListener();

    this.transport = createTransport(this.transportKind(), this.transportHost());
    this.transport.start();
  }

  /**
   * Multi-tab election. Brings up the orchestrator and runs the
   * initial election. Sets `isMultiTabLeader` via the orchestrator's
   * hooks (onInitialLeader / onLatePromote / onDemote).
   *
   * On platforms without BroadcastChannel (Node, jsdom, very old
   * Safari) the orchestrator declares self leader and returns
   * immediately.
   */
  private async initMultiTab(): Promise<void> {
    this.orchestrator = new MultiTabOrchestrator(
      {
        enabled: this.config.multiTab !== false,
        appName: this.config.appName,
      },
      this.subscriptions,
      this.multiTabHooks(),
    );
    await this.orchestrator.init();
  }

  /** Hooks the orchestrator calls back into for inbound multi-tab
   *  events that need engine state. Cases that only touch
   *  subscriptions are dispatched directly by the orchestrator. */
  private multiTabHooks() {
    return {
      onInitialLeader: () => {
        this.isMultiTabLeader = true;
      },
      onLatePromote: () => {
        this.isMultiTabLeader = true;
        void this.onMultiTabPromoted();
      },
      onDemote: () => {
        this.isMultiTabLeader = false;
        this.onMultiTabDemoted();
      },
      onAppliedReceived: (
        changes: ChangeEvent[],
        targetCursor: SyncCursor | undefined,
      ) => {
        // `fromBroadcast: true` suppresses re-broadcast in case a
        // promotion lands between this enqueue and the apply.
        if (changes.length > 0) {
          void this.enqueueApply(changes, targetCursor, { fromBroadcast: true });
        } else if (targetCursor && targetCursor.last_seq > this.cursor.last_seq) {
          this.cursor = targetCursor;
        }
      },
      onReconciledReceived: (
        entity: string,
        upserts: Row[],
        removalIds: string[],
        tombstoneSeq: number,
      ) => {
        void this.enqueueReconcile(entity, upserts, removalIds, tombstoneSeq, {
          fromBroadcast: true,
        });
      },
      onResetReceived: (wipeMutations: boolean) => {
        void this.resetReplicaInner({ wipeMutations });
      },
      onSessionReceived: (resolved: ResolvedSession) => {
        // Funnel through the shared session chain so concurrent triggers
        // (broadcast + local notifySessionChanged) commit in arrival
        // order. Without that the older tenant could win and pin the
        // engine to a stale session.
        void this.applySessionTransition(resolved, /* broadcast */ false);
      },
      onMutationsForwarded: (ops: PendingMutation[]) => {
        for (const op of ops) {
          // Thread the follower's captured `prevRow` so a server
          // rejection of this forwarded update/delete restores the
          // canonical value rather than deleting it. Without it the
          // leader's queue entry has prevRow === undefined, and
          // failPushedMutation's restoreRow(undefined ?? null) would
          // DELETE the leader's still-valid row. The follower's prevRow
          // (its pre-edit value) equals the leader's canonical row, so
          // restoring it is correct on both tabs.
          this.mutations.add(op.change, op.prevRow);
        }
        void this.push();
      },
      onMutationsAcked: (opIds: string[]) => {
        for (const id of opIds) this.mutations.markApplied(id);
        this.mutations.clear();
      },
      onMutationsFailed: (ops: { opId: string; error: string }[]) => {
        // The leader pushed this follower's forwarded mutation and the
        // server rejected it. Roll back the follower's OWN optimistic
        // ghost (the leader already rolled back its copy) — calling
        // markFailed alone left the ghost row stuck in the very tab the
        // user is looking at. failPushedMutation restores prevRow for
        // update/delete and removes the insert ghost, then marks failed.
        for (const op of ops) {
          const m = this.mutations.get(op.opId);
          if (m) {
            this.failPushedMutation(m, op.error);
          } else {
            this.mutations.markFailed(op.opId, op.error);
          }
        }
      },
      onBinaryReceived: (bytes: Uint8Array) => {
        for (const h of this.binaryHandlers) {
          try {
            h(bytes);
          } catch (err) {
            console.warn("[sync] binary handler threw:", err);
          }
        }
      },
      onPeerLeft: (tabId: string) => {
        // Orchestrator already scrubbed CRDT / reactive sub state for
        // the departed tab. Engine cleans up room forwarder sets —
        // a follower that crashed without sending `room-sub-unregister`
        // would otherwise keep the leader's hold (and therefore the WS
        // sub) alive for a room nobody actually wants.
        if (!this.isMultiTabLeader) return;
        for (const [room, set] of [...this.roomForwarders]) {
          if (!set.delete(tabId)) continue;
          if (set.size === 0) {
            this.roomForwarders.delete(room);
            const release = this.leaderForwarderHolds.get(room);
            if (release) {
              this.leaderForwarderHolds.delete(room);
              release();
            }
          }
        }
      },
      onRoomSubRegister: (roomId: string, fromTabId: string) => {
        let set = this.roomForwarders.get(roomId);
        if (!set) {
          set = new Set();
          this.roomForwarders.set(roomId, set);
        }
        const wasFirst = set.size === 0;
        set.add(fromTabId);
        // First forwarder for this room: acquire a leader-internal
        // hold on the registry so the WS sub stays alive as long as
        // any tab (leader or follower) still cares. The hold is a
        // refcount increment with a noop callback — it doesn't fire
        // on snapshots / updates (the leader's own subscribers, if
        // any, get the pulse; the forwarded tab gets the snapshot via
        // the `room-fanout-*` channel). Without this hold, a leader
        // with no local subscribers would never enter the registry
        // and inbound `room-snapshot` for forwarded rooms would drop
        // on the floor.
        if (wasFirst) {
          const noop: RoomSubscriber = () => {};
          const release = this.rooms.register(roomId, noop);
          this.leaderForwarderHolds.set(roomId, release);
        }
      },
      onRoomSubUnregister: (roomId: string, fromTabId: string) => {
        const set = this.roomForwarders.get(roomId);
        if (!set) return;
        set.delete(fromTabId);
        if (set.size > 0) return;
        this.roomForwarders.delete(roomId);
        // Release the leader-internal hold IF no local subscriber on
        // this tab cares either. The registry's refcount will then
        // drop to zero and `room-unsubscribe` ships.
        const release = this.leaderForwarderHolds.get(roomId);
        if (release) {
          this.leaderForwarderHolds.delete(roomId);
          release();
        }
      },
      onRoomFanoutSnapshot: (roomId: string, members: unknown) => {
        if (Array.isArray(members)) {
          this.rooms.applySnapshot(roomId, members as RoomMember[]);
        }
      },
      onRoomFanoutUpdate: (
        roomId: string,
        action: "join" | "leave" | "presence" | "broadcast",
        member: unknown,
        data: unknown,
      ) => {
        this.rooms.applyUpdate(
          roomId,
          action,
          (member ?? undefined) as RoomMember | undefined,
          data,
        );
      },
      onRoomFanoutError: (roomId: string, error: unknown) => {
        const err = (error ?? { code: "UNKNOWN" }) as RoomError;
        this.rooms.applyError(roomId, err);
      },
      onReplayRoomSubs: () => {
        // Follower path: re-broadcast `room-sub-register` for every
        // room we still care about. New leader rebuilds its forwarder
        // set and resends `room-subscribe` on its WS.
        if (this.isMultiTabLeader) return;
        for (const roomId of this.rooms.roomIds()) {
          this.broadcastToTabs({
            type: "room-sub-register",
            room: roomId,
          });
        }
      },
      onEntityObserve: (entity: string) => {
        // Leader path: a follower's useQuery observed this entity. Add
        // it to our reconcile sweep and fetch it now if we have no local
        // rows — the resulting `reconciled` batch is broadcast to every
        // tab, so the follower's view populates. Same shape as the
        // leader half of observeEntity; the `has` guard dedupes against
        // our own interest.
        if (!this.isMultiTabLeader) return;
        if (this.observedEntities.has(entity)) return;
        this.observedEntities.add(entity);
        if (this.isHydrated() && this.store.list(entity).length === 0) {
          void this.reconcile([entity]);
        }
      },
      onReplayObservedEntities: () => {
        // Follower path: re-declare every observed entity to the new
        // leader so its reconcile sweep covers them after a leader flip.
        if (this.isMultiTabLeader) return;
        for (const entity of this.observedEntities) {
          this.broadcastToTabs({ type: "entity-observe", entity });
        }
      },
      onReplayForwardedMutations: () => {
        // Follower path: re-forward our pending batch to the new leader.
        // If we'd forwarded an op to the previous (now-dead) leader, it
        // died before acking, so the op is still pending here as an
        // optimistic ghost with nothing pushing it. The new leader only
        // drains its OWN queue on promotion, so without this re-forward the
        // op is silently lost until our next local write. The leader's
        // `onMutationsForwarded` re-adds by op_id (idempotent) and pushes.
        if (this.isMultiTabLeader) return;
        const pending = this.mutations.pending();
        if (pending.length > 0) {
          this.broadcastToTabs({ type: "mutations", ops: pending });
        }
      },
    };
  }

  /** Leader-side hold tokens for room subs forwarded by followers. The
   *  leader subscribes via its own registry with a no-op callback so
   *  the registry's first-add gate ships `room-subscribe` exactly
   *  once; the release tokens here let us undo that hold when the
   *  last follower stops caring. */
  private leaderForwarderHolds: Map<string, () => void> = new Map();

  /** Seed serverSubs with every subscription this tab currently wants.
   *  Called from both the initial-leader path (subscribes that
   *  happened before start() took the follower branch and broadcast
   *  to a not-yet-running broker, so the registers were lost) and the
   *  late-promotion path (the previous leader owned the subs and we
   *  need to claim them). */
  private seedServerSubsFromLocalInterest(): void {
    this.subscriptions.seedFromLocalInterest();
  }

  /** Late promotion: the previous leader dropped while we were
   *  running as a follower. Take over network ops now AND drain any
   *  pending mutations through push() — a mutation we'd forwarded to
   *  the previous leader may have died with it before the server
   *  acked, and we need to ship it ourselves. op_id makes the
   *  server-side retry idempotent: a previously-applied op returns
   *  `replayed` and we just mark it applied locally. */
  private async onMultiTabPromoted(): Promise<void> {
    if (!this.running) return;
    try {
      // Re-register every locally-wanted server subscription with our
      // own serverSubs. Previous leader had these; now we own the WS,
      // so we need to send the subscribe frames on the next connect.
      this.seedServerSubsFromLocalInterest();
      // Ask the rest of the tabs to re-forward their subs so we can
      // serve them from our new WS. They'll respond via `sub-register`.
      this.broadcastToTabs({ type: "request-sub-replay" });

      await this.refreshResolvedSession();
      await this.pull();
      // Drain the queue — replay every pending op that was either
      // forwarded to a now-dead leader or queued locally while we
      // were a follower. The server's op_id dedupe absorbs duplicates.
      if (this.mutations.pending().length > 0) {
        void this.push();
      }
      this.attachVisibilityListener();
      // Late promotion: build a transport (if one doesn't exist yet — a
      // tab might have spent its whole life as follower with no
      // transport at all) and start it.
      if (!this.transport) {
        this.transport = createTransport(this.transportKind(), this.transportHost());
      }
      this.transport.start();
    } catch {
      /* best-effort — next reconnect cycle catches up */
    }
  }

  /** Demotion: another tab took over as leader. Tear down our
   *  transport (WS socket, ping timer, reconnect timer, poll timer —
   *  whichever applies) and stop driving network ops; the new leader
   *  will broadcast applied changes that our followers-mirror path
   *  picks up. */
  private onMultiTabDemoted(): void {
    if (this.transport) {
      this.transport.stop();
      this.transport = null;
    }
  }

  /** Broadcast a payload to other tabs in this origin. Delegates to
   *  the orchestrator; no-op when the orchestrator isn't running
   *  (SSR-only consumers that never reach `start()`). */
  private broadcastToTabs(payload: unknown): void {
    this.orchestrator?.broadcastRaw(payload);
  }

  private visibilityHandler: (() => void) | null = null;
  private attachVisibilityListener(): void {
    if (this.config.reconcileOnVisibility === false) return;
    if (typeof document === "undefined") return;
    if (this.visibilityHandler) return;
    this.visibilityHandler = () => {
      if (document.visibilityState !== "visible") return;
      if (!this.running) return;
      // Reconcile fires only on tab-becomes-visible; the debounce in
      // reconcile() collapses bursts from rapid background/foreground
      // flips. Pull is enqueued FIRST so the opQueue runs it before
      // reconcile — that way reconcile sees the fresh cursor and
      // doesn't duplicate the cursor-catch-up work pull is already
      // doing. They're serial via the queue, not concurrent.
      void this.pull();
      void this.reconcile();
    };
    document.addEventListener("visibilitychange", this.visibilityHandler);
  }

  /**
   * Serialize a batch of change applies behind any in-flight applies, and
   * advance the cursor monotonically when the batch lands. Both the WS
   * onmessage path and pull() funnel through here so seq 3's persistence
   * can't race ahead of seq 2's. The returned promise resolves after
   * THIS batch is applied (not after later batches), so a caller awaiting
   * pull() still completes deterministically.
   *
   * Per-event monotonic filter: re-applies of an already-seen seq are
   * skipped before touching the store. Without that, a retransmit
   * (WS + pull window overlap) would have us run applyChange twice
   * against the local store.
   */
  private enqueueApply(
    changes: ChangeEvent[],
    targetCursor?: SyncCursor,
    opts: { fromBroadcast?: boolean; isPull?: boolean } = {},
  ): Promise<void> {
    // Snapshot fence: while a from-zero snapshot is in flight, hold live WS
    // frames + tab broadcasts so they can't advance the cursor past the
    // snapshot's rows and filter them out (see `snapshotHold`). The pull's own
    // apply (`isPull`) is exempt — it IS the snapshot. Held events are replayed
    // in arrival (≈seq) order once the snapshot lands. Synchronous + before the
    // queue chain so held events never interleave into the applyQueue.
    if (this.snapshotHold !== null && !opts.isPull) {
      this.snapshotHold.push(...changes);
      return Promise.resolve();
    }
    const prev = this.applyQueue;
    const next = prev.then(async () => {
      // Per-event monotonic filter: re-applies of an already-seen seq
      // are skipped before touching the store. Without that, a
      // retransmit (WS + pull window overlap) would have us run
      // applyChange twice against the local store.
      const filtered = changes.filter(
        (c) => typeof c.seq === "number" && c.seq > this.cursor.last_seq,
      );
      if (filtered.length > 0) {
        const durable = await this.store.applyChangesAsync(filtered);
        // A row in this batch didn't reach disk (quota / abort). Latch
        // the degraded flag so we never persist a cursor ahead of the
        // durable replica — the next cold start must re-pull this gap.
        if (!durable) this.persistDegraded = true;
      }
      // Pick the cursor target. Explicit `targetCursor` (from pull) wins
      // — pull's response carries the server's authoritative current_seq
      // even when no changes landed in this window. Otherwise derive
      // from the last applied seq.
      const candidate =
        targetCursor ??
        (filtered.length > 0
          ? { last_seq: filtered[filtered.length - 1].seq }
          : null);
      if (candidate && candidate.last_seq > this.cursor.last_seq) {
        // In-memory cursor ALWAYS advances — live sync stays correct.
        this.cursor = candidate;
        // The on-disk cursor only advances while persistence is healthy.
        // Once degraded, freezing it keeps disk self-consistent (cursor
        // never exceeds the rows actually written) so restart re-pulls.
        if (this.persistence && !this.persistDegraded) {
          await this.persistence.saveCursor(this.cursor);
        }
      }
      // Multi-tab: leader fans the batch out so follower replicas
      // converge without their own WS. Skip when we ourselves
      // RECEIVED this batch from another tab — otherwise a tab that
      // was promoted between receiving and applying would re-broadcast
      // its own copy, and even though the seq filter dedupes on
      // arrival the round-trip is wasted bandwidth.
      if (
        this.isMultiTabLeader &&
        !opts.fromBroadcast &&
        filtered.length > 0
      ) {
        this.broadcastToTabs({
          type: "applied",
          changes: filtered,
          targetCursor: candidate ?? undefined,
        });
      }
    });
    // Errors stay scoped to this batch — don't poison the chain for
    // future applies.
    this.applyQueue = next.catch(() => {});
    return next;
  }

  /**
   * Reconcile path. Routes through the same applyQueue as WS/pull so
   * a reconcile batch can't interleave with a fresher change event
   * mid-apply — but reconcile carries no real seqs, so the seq filter
   * and cursor advance from `enqueueApply` are deliberately absent.
   * Pending mutations are protected upstream (in applyEntityReconcile)
   * before the batch is ever built.
   */
  private enqueueReconcile(
    entity: string,
    upserts: Row[],
    removalIds: string[],
    tombstoneSeq: number,
    opts: { fromBroadcast?: boolean } = {},
  ): Promise<void> {
    const prev = this.applyQueue;
    const next = prev.then(async () => {
      await this.store.applyReconcileBatch(
        entity,
        upserts,
        removalIds,
        tombstoneSeq,
      );
      // Leader fans the reconcile batch out so each follower can
      // converge without its own fetch. Suppress when we ourselves
      // received this batch via the channel (promotion mid-flight
      // would otherwise echo it).
      if (this.isMultiTabLeader && !opts.fromBroadcast) {
        this.broadcastToTabs({
          type: "reconciled",
          entity,
          upserts,
          removalIds,
          tombstoneSeq,
        });
      }
    });
    this.applyQueue = next.catch(() => {});
    return next;
  }

  /** Stop the sync engine. */
  stop(): void {
    this.running = false;
    if (this.transport) {
      this.transport.stop();
      this.transport = null;
    }
    if (this.visibilityHandler && typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", this.visibilityHandler);
      this.visibilityHandler = null;
    }
    if (this.orchestrator) {
      this.orchestrator.stop();
      this.orchestrator = null;
    }
    this.setConnectionStatus("offline");
  }


  /**
   * Drop local cursor + store + notify. Safe to call from any state.
   * Used by:
   *  - the 410 RESYNC_REQUIRED handler (server says our cursor is stale)
   *  - the identity-change detector in pull() (new auth = new visible set)
   *  - callers that need to force a clean re-pull (tests, sign-out flows)
   *
   * Does NOT issue the subsequent pull — callers decide when to re-pull.
   * That keeps the lifecycle explicit: a caller can reset, swap config,
   * then pull.
   *
   * Clears IndexedDB too. Without that, locals that should have been
   * deleted server-side (e.g. another client deleted rows while this tab
   * was closed, then this tab's cursor 410'd) survived on disk and got
   * rehydrated on the next page load — phantom rows that no purge of
   * in-memory state could fix.
   */
  async resetReplica(opts: { wipeMutations?: boolean } = {}): Promise<void> {
    // Public callers go through the queue so a reset can't race with
    // an in-flight pull / push / reconcile. Internal callers that
    // already hold the queue slot use `resetReplicaInner` directly.
    return this.opQueue.enqueue("reset", () => this.resetReplicaInner(opts));
  }

  /**
   * Wipe the hydrated replica when it belongs to a different identity than
   * the currently-resolved session — the shared-browser / account-switch
   * leak that surfaces only across a page reload (a live token flip is
   * already caught by `observeToken` in pull()). No-op when nothing was
   * hydrated, when the persisted identity is unknown (a pre-tag replica —
   * we tag it on this run instead of wiping, so the protection kicks in
   * from the next reload), or when it already matches.
   */
  private async guardReplicaIdentity(): Promise<void> {
    if (!this._hadCachedReplica) return;
    const prev = this._replicaIdentity;
    if (prev === undefined) return;
    const now = this.session.resolved().userId;
    if (prev === now) return;
    // Identity flipped across the reload. Drop the prior identity's rows.
    // Keep pending offline writes ONLY for guest→user (the anonymous-merge
    // login: the server reassigns the guest's rows to the new user, and the
    // queued writes should re-push under the user). For any other flip
    // (user A → user B, user → guest) discard them so one identity's
    // unsynced writes never push under another.
    const isGuestToUser =
      typeof prev === "string" &&
      prev.startsWith("guest_") &&
      typeof now === "string" &&
      !now.startsWith("guest_");
    await this.resetReplica({ wipeMutations: !isGuestToUser });
    this.persistReplicaIdentity();
  }

  /**
   * Record the identity that owns the current replica (in memory + on disk)
   * so a later cold start can detect an account switch. Fire-and-forget —
   * the tag is a hint; a missed write just degrades to "tag unknown" which
   * the guard treats conservatively (no wipe, re-tag next run).
   */
  private persistReplicaIdentity(): void {
    const id = this.session.resolved().userId;
    this._replicaIdentity = id;
    if (this.persistence && !this.persistDegraded) {
      void this.persistence.saveIdentity(id);
    }
  }

  /**
   * Drop the local replica and pull fresh. `wipeMutations` decides the
   * fate of the durable offline write queue:
   * - `false` (default, 410 RESYNC, SAME user): KEEP pending writes —
   *   they survive the snapshot refresh and re-push under the same
   *   session.
   * - `true` (token/tenant flip, DIFFERENT identity): DROP them — the
   *   queued writes belong to the outgoing identity and must never be
   *   replayed as the incoming one (cross-identity write leak).
   */
  private async resetReplicaInner(
    opts: { wipeMutations?: boolean } = {},
  ): Promise<void> {
    const wipeMutations = opts.wipeMutations === true;
    this.cursor = { last_seq: 0 };
    this.store.clearAll();
    // The replica was just wiped and will re-pull from 0 (org switch, identity
    // flip, or a 410 cursor reset). Re-enter "loading" until that re-pull lands
    // rows — otherwise switching to another org flashes an empty list for the
    // seconds the snapshot takes. Re-arm the fallback so it can't pin; the
    // arriving rows (populated org) or the deadline (empty org) re-settle it.
    this._initialSyncSettled = false;
    this.armInitialSyncFallback();
    // Disk is about to be wiped + re-pulled from 0, so any prior
    // persist degradation is moot — start the durability invariant
    // fresh. (If the fresh snapshot also fails to persist, enqueueApply
    // re-latches the flag.)
    this.persistDegraded = false;
    if (wipeMutations) {
      // Identity flip: discard the outgoing identity's pending offline
      // writes (and persist the empty queue to disk via the mutation
      // backend). persistence.clear() deliberately leaves MUTATIONS_STORE
      // alone for the 410 path, so this is the only site that drops them.
      this.mutations.clearAll();
    }
    // The cache is now empty. The next pull will start from 0 and
    // return a full snapshot — that's a true cold start, so the
    // onConnected fast-path may skip the post-pull reconcile. Without
    // this flip, a sign-out → sign-in inside the same tab would
    // forever re-run reconcile after every pull because
    // `_hadCachedReplica` was set to true at start() time and never
    // cleared.
    this._hadCachedReplica = false;
    if (this.persistence) {
      try {
        await this.persistence.clear();
        await this.persistence.saveCursor(this.cursor);
        // clear() wiped the cursor store (identity tag included). The
        // replica is about to be re-pulled for the CURRENT identity, so
        // re-tag it to match — otherwise the on-disk tag reads "unknown"
        // until the next cold-start pull re-records it.
        const id = this.session.resolved().userId;
        this._replicaIdentity = id;
        await this.persistence.saveIdentity(id);
      } catch {
        /* best-effort */
      }
    }
    // Leader broadcasts the reset so follower replicas wipe their
    // own copies in lockstep — otherwise a follower keeps stale
    // rows under the old identity until its own pull catches up. The
    // `wipeMutations` flag rides along so followers make the same
    // keep-vs-drop decision for THEIR forwarded offline writes.
    if (this.isMultiTabLeader) {
      this.broadcastToTabs({ type: "reset", wipeMutations });
    }
  }

  /**
   * localStorage key for the auth token, namespaced by appName. Matches
   * the key the React package's `configureClient` writes to so the sync
   * engine and the hooks agree on where the token lives.
   */
  private tokenStorageKey(): string {
    const app = this.config.appName || "default";
    return app === "default" ? "pylon_token" : `pylon:${app}:token`;
  }

  /** Current auth token from config or the storage adapter. Null when neither has one. */
  private currentToken(): string | null {
    if (this.config.token) return this.config.token;
    return this.storage.get(this.tokenStorageKey());
  }

  /**
   * Call a Pylon Action / Mutation by name.
   *
   * Wraps `POST /api/fn/<name>` with the engine's bearer/cookie auth
   * AND observes the `X-Pylon-Change-Seq` response header. If the
   * server reports the action generated change events that the local
   * replica hasn't seen yet, the engine immediately fires a one-shot
   * pull — closing the latency window between the HTTP response
   * landing here and the WS broadcast of the same events arriving.
   *
   * App code that uses this method no longer needs the
   * "after-mutation refetch()" workaround pattern (see pylon-cloud's
   * domains/page.tsx, pre-2026-05-17, which called refetch() four
   * times for exactly this reason).
   *
   * Throws (`Error & {status, code?}`) on non-2xx with the server's
   * error envelope. Returns the parsed JSON response.
   */
  async fn<T = unknown>(name: string, args: unknown = {}): Promise<T> {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      // Guard against accidentally embedding URL slashes/query in the
      // function name. The framework validates server-side too, but
      // failing here is a clearer DX error message.
      throw new Error(`Invalid function name: ${JSON.stringify(name)}`);
    }
    return this.requestWithChangeSync<T>(
      "POST",
      `/api/fn/${name}`,
      args,
    );
  }

  /** Shared by `fn()` and any future entity-mutation wrappers. POSTs
   *  through the central transport, observes `X-Pylon-Change-Seq`,
   *  and triggers a one-shot pull when the server says it produced
   *  events past our local cursor. The pull short-circuits cheaply
   *  (`{changes:[]}`) if WS broadcast already caught us up — so the
   *  worst case is one extra in-flight pull per mutation, never a
   *  stale render. */
  private async requestWithChangeSync<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    return pylonFetch<T>(
      {
        baseUrl: this.config.baseUrl,
        getToken: () =>
          this.config.token ??
          this.storage.get(this.tokenStorageKey()) ??
          undefined,
        onChangeSeq: (seq) => {
          if (seq > this.cursor.last_seq) {
            void this.pull();
          }
        },
      },
      path,
      {
        method: method as "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
        json: body,
      },
    );
  }

  /** Pull changes from the server. Coalesces concurrent callers via
   *  the op queue and serializes against push / reconcile / reset, so
   *  the cursor can't be read mid-reset and the change-log delta can't
   *  interleave with a sweeping reconcile. The 410 RESYNC retry path
   *  recurses into `pullInner` directly to avoid self-deadlock on the
   *  queue. */
  async pull(): Promise<void> {
    await this.opQueue.enqueue("pull", () => this.pullInner());
    // A completed leader pull is a server-confirmed view — settle the
    // initial-sync signal so `useQuery`'s loading drops (cold start, and the
    // re-sync after a replica wipe / org switch). Followers no-op in
    // pullInner and settle via the leader's broadcasts or the fallback, so we
    // gate on leadership here to avoid an empty-state flash on a follower.
    if (this.isMultiTabLeader) this.markInitialSyncSettled();
  }

  private async pullInner(): Promise<void> {
    // Followers don't talk to the network — the leader broadcasts
    // every applied change over the multi-tab channel, and our local
    // applyQueue picks it up there.
    if (!this.isMultiTabLeader) return;
    // Identity change detection. If the token flipped since the last pull
    // (anonymous → signed in, user A → user B, signed in → signed out),
    // the server's visible set changed under us and the cursor we saved
    // reflects the previous identity. Reset before pulling so we rebuild
    // the replica from seq=0 under the new identity.
    const { tokenChanged } = this.session.observeToken(this.currentToken());
    if (tokenChanged) {
      // We're holding the "pull" slot in the op queue — bypass the
      // queue's reset path to avoid self-deadlock. Identity flipped, so
      // wipe the old identity's pending offline writes.
      await this.resetReplicaInner({ wipeMutations: true });
      // Token flipped → the cached tenant is for the previous user. Pull
      // the fresh session in parallel with the cursor catch-up below.
      void this.refreshResolvedSession();
    }

    // Capture whether this pull started from cursor=0 BEFORE the
    // snapshot loop mutates the cursor. On successful exhaustion the
    // WS onConnected hook reads the flag to skip the redundant
    // bootstrap reconcile (the snapshot path already returned every
    // policy-visible row, per-entity refetch right after is waste).
    const startedFromZero = this.cursor.last_seq === 0;
    // A from-zero pull is a SNAPSHOT — open the live-event hold for its whole
    // (possibly multi-page) duration so a racing WS frame can't leapfrog the
    // cursor and filter the snapshot rows out. Nested pulls (delta tail /
    // has_more, 410 recursion) run at a non-zero cursor → they don't touch
    // this, and their applies pass `isPull` so they're never held.
    if (startedFromZero) this.snapshotHold = [];
    try {
      // Snapshot pagination: when the cursor is 0 and the server's
      // table is larger than a single batch, the response carries
      // `snapshot_after` for the next page. Loop until exhausted
      // BEFORE returning so a fresh client always observes a
      // consistent full snapshot, not a 1k-row prefix it mistakes
      // for the whole replica.
      let snapshotAfter: string | undefined;
      let firstPass = true;
      while (firstPass || snapshotAfter) {
        firstPass = false;
        // `snapshot_after` is an OPAQUE cursor the server already URL-encoded
        // (it `url_encode`s the JSON payload). It MUST be appended raw — running
        // it back through URLSearchParams would double-encode it, the server's
        // single `url_decode` would leave it still-encoded, `serde_json::from_str`
        // would fail, and the server would treat the page as "no resume" and
        // restart the snapshot from row 0 — an infinite re-snapshot loop for any
        // table larger than one page (SNAPSHOT_BATCH_LIMIT rows). `since` is a
        // plain integer, so it's safe to inline.
        let query = `since=${this.cursor.last_seq}`;
        if (snapshotAfter) {
          query += `&snapshot_after=${snapshotAfter}`;
        }
        const resp = await this.request<
          PullResponse & { snapshot_after?: string | null }
        >("GET", `/api/sync/pull?${query}`);
        await this.enqueueApply(resp.changes, resp.cursor, { isPull: true });
        // `snapshot_after` is only set when the server is mid-snapshot.
        // Continue paginating in the same loop iteration so we don't
        // leave a fresh client with a partial replica.
        snapshotAfter = resp.snapshot_after ?? undefined;
        // The change-log tail also paginates via `has_more` — drain it
        // by recursing into `pullInner` directly. We are INSIDE the
        // `pull` op-queue slot right now; calling the public `pull()`
        // would re-enqueue under the same "pull" key, which coalesces
        // to the promise we're currently running inside (op-queue.ts
        // deletes the key only after `fn` resolves) and `await` it →
        // permanent self-deadlock that bricks the entire pull path for
        // the session. This is the exact hazard the 410 handler avoids;
        // `pullInner` re-reads `this.cursor.last_seq` (already advanced
        // by enqueueApply) so the recursion resumes at the right cursor.
        if (!snapshotAfter && resp.has_more) {
          await this.pullInner();
          break;
        }
      }
      // Clear the resync circuit breaker ONLY on a successful DELTA
      // pull — one that started from a real, non-zero cursor the server
      // honored. A snapshot pull from cursor=0 succeeding does NOT prove
      // our cursor is stable, so it must NOT clear the breaker.
      //
      // Why this matters (the bug this replaced): the reset used to fire
      // on every successful page, including the cursor=0 snapshot that a
      // 410 triggers. So a stale-cursor 410 → full snapshot → fresh-
      // cursor 410 ping-pong — a client bouncing between cluster
      // instances whose in-memory change logs diverge, with no shared
      // persistent log to serve the delta — reset the breaker every
      // cycle. The exponential backoff below could never engage, and the
      // client re-ran a full `select *` snapshot of EVERY entity roughly
      // every 3 seconds, indefinitely. That drove a ~280GB PlanetScale
      // egress bill. Resetting only on a delta means repeated resyncs
      // escalate into the backoff instead of melting egress.
      if (!startedFromZero) {
        this.consecutive_410s = 0;
      }
      // Snapshot+tail loop exhausted without throwing: if we started
      // from cursor=0 we just hydrated the full replica from server
      // truth. Record it so onConnected skips the reconcile that would
      // otherwise re-fetch every entity via cursor pagination.
      this.lastPullStartedFromZero = startedFromZero;
      // Snapshot landed cleanly → replay the live events we held during it, in
      // arrival (≈seq) order. They filter against the now-correct cursor
      // (snapshot_seq), so events newer than the snapshot apply and older ones
      // (already in the snapshot) are deduped. Clearing `snapshotHold` first
      // means this replay applies normally (it isn't re-held).
      if (startedFromZero && this.snapshotHold) {
        const held = this.snapshotHold;
        this.snapshotHold = null;
        if (held.length > 0) await this.enqueueApply(held);
      }
    } catch (err) {
      // Swallow network + transient errors so the poll/reconnect loop
      // keeps trying — but on 429 bump the backoff counter so the next
      // reconnect waits noticeably longer. Without this, a rate-limited
      // pull triggers onclose → scheduleReconnect → pull → 429 in a
      // tight loop that the server reads as abuse.
      const status = (err as { status?: number })?.status;
      if (status === 429) {
        // Push the next reconnect noticeably further out so a rate-
        // limited pull doesn't drive a tight 429 / reconnect / pull
        // / 429 loop the server reads as abuse. The +3 attempts skips
        // straight to a longer backoff window.
        this.transport?.bumpReconnect(3);
      }
      // 410 RESYNC_REQUIRED: cursor is from a previous server lifetime, or
      // it fell off the retention window. Drop local state + cursor and
      // re-pull from seq=0. The server replays all current entity rows as
      // seed events on startup so the fresh pull reconstructs state.
      //
      // Circuit breaker. The first resync in an episode snapshots
      // immediately (good UX: a cursor that genuinely fell off retention
      // recovers in one round trip). But a SECOND 410 with no successful
      // delta pull in between means re-snapshotting isn't converging —
      // the cursor we just minted from the snapshot is itself stale
      // (instance ping-pong, or a server that can't serve our delta). At
      // that point each snapshot is a full `select *` of every entity,
      // so we MUST back off instead of looping. The breaker only clears
      // on a successful delta pull (see the `!startedFromZero` reset
      // above) — NOT on the snapshot itself, which is what made this
      // loop unbounded and burned ~280GB of egress.
      if (status === 410) {
        const attempt = this.consecutive_410s;
        this.consecutive_410s += 1;
        if (attempt === 0) {
          // First resync of the episode — snapshot now. Bypass the queue
          // (we ARE the pull op holding the slot; the public pull()
          // would re-enqueue and share our own promise back → deadlock).
          await this.resetReplicaInner();
          await this.pullInner();
        } else {
          // Snapshotted once and still 410 → not converging. Back off
          // exponentially instead of re-snapshotting. Clears only when a
          // delta pull finally succeeds (cursor stabilised).
          const delayMs = Math.min(30_000, 1000 * 2 ** Math.min(attempt, 5));
          console.warn(
            `[pylon] persistent 410 RESYNC_REQUIRED (attempt ${attempt + 1}); backing off ${delayMs}ms`,
          );
          setTimeout(() => {
            // Retry after the delay; a delta success resets the counter,
            // a repeat 410 extends the backoff (no snapshot).
            void this.pull();
          }, delayMs);
        }
      }
    } finally {
      // Snapshot pull failed (network error / 410 mid-fetch): DISCARD any
      // still-held live events rather than applying them. The cursor stays at 0
      // so the retry resnapshots and re-covers them; applying them here would
      // advance the cursor and turn the retry into a gappy delta. On success
      // the try already drained + nulled the hold, so this is a no-op there.
      // Nested non-zero pulls never set `snapshotHold`, so this only fires for
      // the from-zero snapshot that owns it.
      if (startedFromZero && this.snapshotHold !== null) this.snapshotHold = null;
    }
  }

  /** Consecutive 410 RESYNC_REQUIRED responses since the last successful
   *  pull. Used by the circuit breaker in pull() to bound the retry
   *  storm against a misconfigured server. Resets to 0 on any pull
   *  that doesn't throw a 410. */
  private consecutive_410s = 0;

  /** Consecutive TRANSIENT push failures (offline / 5xx / 429 / 401)
   *  since the last server response. Drives the exponential backoff on
   *  the retry of a transient-failed push so an offline tab doesn't
   *  hot-loop. Reset to 0 the moment the server returns any response. */
  private pushFailureCount = 0;

  /** Set by pullInner whenever the just-completed pull started with
   *  `cursor.last_seq === 0` (cold load OR post-reset). The WS
   *  onConnected hook reads this to skip the reconcile() that would
   *  otherwise fire immediately after the bootstrap pull — the
   *  snapshot path of pull already returned every row visible under
   *  current policy, so per-entity reconcile fetches right after are
   *  pure waste (~300ms on the critical path). One-shot: the flag is
   *  cleared on read so a subsequent reconnect-after-disconnect still
   *  runs reconcile normally. */
  private lastPullStartedFromZero = false;

  /** Timestamp of the last `reconcile()` invocation. Used to debounce —
   *  reconcile runs on connect, WS reconnect, AND visibility-change, so
   *  a quick tab-flick after a normal reconnect shouldn't refetch every
   *  entity twice within seconds. Configurable via `reconcileMinIntervalMs`. */
  private lastReconcileAt = 0;

  /** Entities the app has subscribed to via `useQuery` / `useQueryOne`,
   *  even ones the local replica has zero rows for. The reconcile
   *  safety net defaults to `store.entityNames()` — entities with at
   *  least one local row — so a server row in a NEVER-cached entity (a
   *  row created on another surface, or a freshly-added entity) stayed
   *  invisible until a full snapshot / cache clear: `useQuery` reads
   *  the local store and a delta `pull()` can't recover a row created
   *  before the cursor. Tracking observed entities lets the no-arg
   *  reconcile sweep them too. See `observeEntity`. */
  private observedEntities = new Set<string>();

  /** Per-entity count of CONSECUTIVE 403s seen during reconcile, reset on
   *  any successful fetch. A single 403 can be transient (a bearer caught
   *  mid-refresh, a momentary policy blip) and must NOT wipe the local cache;
   *  we only drop an entity's rows after two in a row. See `reconcile`. */
  private reconcile403Streak = new Map<string, number>();

  /**
   * Reconcile the local replica against server truth.
   *
   * For each entity that has at least one local row, fetch the
   * authoritative row set from `/api/entities/<entity>` (already
   * policy-filtered) and apply the diff:
   *
   * - Local rows whose id is missing from the server set → removed.
   * - Server rows whose JSON differs from local → overwritten.
   * - Server rows the local replica doesn't have → inserted.
   *
   * This is the safety net the WS / pull path can't provide on its own:
   *
   *   - Deletes made by other surfaces (Mac SDK, server-side actions,
   *     admin tools) while this client was offline can fall off the
   *     in-memory ChangeLog before this client reconnects. The pull
   *     then returns an empty diff and the local phantom rows persist
   *     forever. Reconcile catches them.
   *
   *   - Mutations broadcast on a sibling Fly machine (multi-instance
   *     deploys, autoscaled apps) never reach this WS. Reconcile is
   *     the only mechanism that observes them.
   *
   *   - Events broadcast in the brief window between a pull completing
   *     and the WS opening get dropped because the WS has no replay-
   *     on-connect; reconcile makes those eventually-consistent.
   *
   * Debounced via `lastReconcileAt` so a flurry of triggers
   * (reconnect + visibility-change firing back-to-back) coalesces to
   * one network round per entity.
   *
   * Pass an explicit entity list to scope the reconcile (callers like
   * `db.useQueryOne` that know what they care about). When called with
   * no arg, every entity with local rows OR observed via `useQuery`
   * (see `observeEntity`) is checked.
   */
  /**
   * Register interest in an entity — called by `useQuery` /
   * `useQueryOne` on mount. Two effects:
   *
   *   1. Adds the entity to the reconcile sweep so the safety net
   *      covers it even with zero local rows (see `observedEntities`).
   *   2. The FIRST time an entity is observed while the replica is
   *      hydrated and that entity is locally empty, fires a one-shot
   *      scoped reconcile so a server row this client never cached
   *      appears on page-open — instead of waiting for the next
   *      reconnect / visibility-change trigger. Bounded: at most once
   *      per entity per engine (the `observedEntities` guard).
   *
   * Genuinely-empty entities just pay one cheap policy-filtered fetch;
   * entities where the client missed an insert get the row back.
   */
  observeEntity(entity: string): void {
    if (this.observedEntities.has(entity)) return;
    this.observedEntities.add(entity);
    if (!this.isMultiTabLeader) {
      // Follower: only the leader talks to the network. Forward the
      // interest so the LEADER adds this entity to its reconcile sweep
      // and fetches any server row we never cached — then converge via
      // the `reconciled` broadcast. Without the forward, a follower's
      // useQuery on a never-cached entity renders empty forever (the
      // leader never sweeps an entity it has no local rows for and was
      // never told a peer cares about).
      this.broadcastToTabs({ type: "entity-observe", entity });
      return;
    }
    if (this.isHydrated() && this.store.list(entity).length === 0) {
      // Scoped reconcile bypasses the no-arg debounce and reuses the
      // session-flip / cursor-drift guards in reconcileInner.
      void this.reconcile([entity]);
    }
  }

  async reconcile(entities?: string[]): Promise<void> {
    const minIntervalMs = this.config.reconcileMinIntervalMs ?? 2_000;
    const now = Date.now();
    if (entities === undefined && now - this.lastReconcileAt < minIntervalMs) {
      return;
    }
    // Coalesce concurrent reconciles to a single op via the queue's
    // keyed dedupe — multiple callers in the same tick share one fetch.
    // Reconcile waits behind any in-flight pull / refresh / push so it
    // can't apply rows captured under a stale session.
    return this.opQueue.enqueue("reconcile", async () => {
      try {
        await this.reconcileInner(entities);
      } finally {
        this.lastReconcileAt = Date.now();
      }
    });
  }

  private async reconcileInner(entities?: string[]): Promise<void> {
    // Same reasoning as pullInner: the leader reconciles, broadcasts
    // results, and follower replicas converge via the channel.
    if (!this.isMultiTabLeader) return;
    // Sweep entities with local rows PLUS entities the app has observed
    // via useQuery (even when empty locally). Without the observed set,
    // a server row in a never-cached entity is never reconciled and
    // stays invisible until a full snapshot.
    const names =
      entities ??
      [...new Set([...this.store.entityNames(), ...this.observedEntities])];
    if (names.length === 0) return;
    // Tombstone seq for any local row the server doesn't return. Using
    // the current cursor means future inserts (which have higher seqs)
    // bypass the tombstone — re-creation server-side still propagates.
    const tombstoneSeq = this.cursor.last_seq;
    // Fan out the per-entity fetches in parallel. Bootstrap reconcile
    // used to serialize 5 entities × ~60ms each → 300ms of dead time
    // on the critical path before channels render. The per-entity
    // drift checks (cursor + session signature) are captured inside
    // each task's closure, so each entity still bails individually
    // if its OWN fetch raced a WS event or a session flip — parallel
    // fan-out doesn't weaken either guard.
    await Promise.all(
      names.map(async (entity) => {
        // Capture cursor + resolved session BEFORE the fetch so we can
        // detect drift mid-reconcile. Two distinct races:
        //
        //   1. Cursor moves: a WS event for this (or another) entity
        //      landed while the page-paginated fetch was in flight. Our
        //      snapshot is stale; applying it would clobber the fresher
        //      WS-delivered row.
        //
        //   2. Session flips: the resolved tenant/user changed while
        //      the fetch was in flight (e.g., the app called
        //      /api/auth/select-org just after we issued the fetch).
        //      The server filtered the response under the OLD tenant
        //      context, so applying the result would tombstone rows
        //      that ARE visible under the NEW tenant. This is the
        //      "dashboard flashes data away on first load" bug — the
        //      engine starts before the app calls selectOrg, fetches
        //      under tenant=null, returns 0 rows, then the apply pass
        //      nukes every locally-cached row. Skip the apply when
        //      the session signature changed; the next reconcile
        //      (triggered by session-changed envelope) will re-fetch
        //      under the new context.
        const cursorBeforeFetch = this.cursor.last_seq;
        const sessionBeforeFetch = this.session.signature();
        let serverRows: Row[];
        let fetchTruncated: boolean;
        try {
          const fetched = await this.fetchEntityRows(entity);
          serverRows = fetched.rows;
          fetchTruncated = fetched.truncated;
        } catch (err) {
          // Network errors are expected (offline, transient 5xx). Skip
          // this entity; the next reconcile trigger will retry.
          const status = (err as { status?: number })?.status;
          if (status === 404) {
            // Entity removed from the manifest — definitive. Drop its rows.
            this.reconcile403Streak.delete(entity);
            await this.dropEntity(entity, tombstoneSeq);
            return;
          }
          if (status === 403) {
            // A 403 can be TRANSIENT — a bearer caught mid-refresh, a tenant
            // flip that raced the fetch, or a momentary server-side policy
            // blip. Nuking every local row on the first one makes the data
            // flash away and reappear next reconcile (the "rows vanish on
            // load" bug, on the error path this time). Two gates before we
            // drop:
            //   1. If the session flipped during the fetch, the 403 reflects
            //      the OLD context — never drop; the next reconcile under the
            //      new session decides (mirrors the success-path guard below).
            if (this.session.signature() !== sessionBeforeFetch) {
              return;
            }
            //   2. Otherwise require TWO consecutive 403s for this entity, so
            //      a one-off blip can't wipe the cache. A successful fetch
            //      (below) resets the streak.
            const streak = (this.reconcile403Streak.get(entity) ?? 0) + 1;
            if (streak < 2) {
              this.reconcile403Streak.set(entity, streak);
              return;
            }
            this.reconcile403Streak.delete(entity);
            await this.dropEntity(entity, tombstoneSeq);
          }
          return;
        }
        // Successful fetch — the entity is readable, so any prior 403 streak
        // is broken (even if a drift guard below makes us skip the apply).
        this.reconcile403Streak.delete(entity);
        if (this.cursor.last_seq !== cursorBeforeFetch) {
          // Cursor moved during fetch — at least one WS event for this
          // (or another) entity landed and might have a fresher value
          // for a row our snapshot just captured. Bail out for this
          // entity; reconcile() is triggered again on visibility-change
          // and reconnect, and the WS event already carried the latest
          // state for the affected row.
          return;
        }
        if (this.session.signature() !== sessionBeforeFetch) {
          // Session changed (token flipped, tenant switched, user
          // signed out → in, etc.). The rows we fetched reflect the
          // OLD session's policy view; applying them now would
          // tombstone rows visible under the NEW session. Bail and let
          // the session-changed envelope drive the next reconcile.
          return;
        }
        await this.applyEntityReconcile(
          entity,
          serverRows,
          tombstoneSeq,
          fetchTruncated,
        );
      }),
    );
  }

  /** Fetch every row for an entity. Uses cursor pagination so big tables
   *  don't blow past server-side limits; loops until `has_more` is false
   *  or a safety cap is hit. Returns `truncated: true` when the cap was hit
   *  with rows STILL on the server — the fetched set is then INCOMPLETE and
   *  the caller must not infer deletions from it (see applyEntityReconcile). */
  private async fetchEntityRows(
    entity: string,
  ): Promise<{ rows: Row[]; truncated: boolean }> {
    const out: Row[] = [];
    let cursor: string | null = null;
    // 200 pages × 100 per page = 20k rows. A `sync:true` entity larger than
    // this should switch to `sync: false` + search/by-id — see useInfiniteQuery.
    for (let page = 0; page < 200; page++) {
      const qs: string = cursor
        ? `?limit=100&after=${encodeURIComponent(cursor)}`
        : `?limit=100`;
      const resp: {
        data: Row[];
        next_cursor: string | null;
        has_more: boolean;
      } = await this.request("GET", `/api/entities/${entity}/cursor${qs}`);
      for (const row of resp.data) out.push(row);
      if (!resp.has_more || !resp.next_cursor) {
        return { rows: out, truncated: false };
      }
      cursor = resp.next_cursor;
    }
    // Exhausted the page cap while the server still had more rows: the set
    // is incomplete. Surface it so reconcile upserts-only instead of
    // deleting every row past the cap.
    return { rows: out, truncated: true };
  }

  private async applyEntityReconcile(
    entity: string,
    serverRows: Row[],
    tombstoneSeq: number,
    truncated: boolean,
  ): Promise<void> {
    // Invariant: rows with in-flight or failed mutations are
    // off-limits to reconcile. Neither the upsert branch nor the
    // tombstone branch may touch them. A hydrated offline mutation
    // that hasn't been pushed yet would otherwise look like a phantom
    // local-only row and get tombstoned before push has a chance to
    // ship it.
    // Test: `hydrated_offline_mutations_survive_startup_reconcile`.
    const pendingKeys = this.mutations.pendingRowKeys();
    const serverIds = new Set<string>();
    const upserts: Row[] = [];
    for (const row of serverRows) {
      const id = (row as { id?: unknown }).id;
      if (typeof id !== "string" || id.length === 0) continue;
      serverIds.add(id);
      if (pendingKeys.has(`${entity}/${id}`)) continue;
      const local = this.store.get(entity, id);
      if (!local || rowsDiffer(local, row)) {
        upserts.push(row);
      }
    }
    // Removals: every local row whose id isn't in the server set is
    // stale. The reconcile primitive tombstones at `tombstoneSeq` so
    // future legitimate re-creations (with strictly greater seqs)
    // still flow through.
    //
    // BUT only when the server set is COMPLETE. A truncated fetch (entity
    // larger than the 20k reconcile page cap) returns just the first pages —
    // a row absent from it may simply live on an un-fetched page, not be
    // deleted. Inferring deletion here would silently tombstone every row
    // past the cap (the rows flap: full resync re-adds them, next reconcile
    // deletes them again). Upsert only; never remove from an incomplete set.
    const removalIds: string[] = [];
    if (truncated) {
      console.warn(
        `[reconcile] "${entity}" exceeds the ${20_000}-row client-replication ` +
          `cap; skipping stale-row cleanup to avoid deleting un-fetched rows. ` +
          `Mark large server-queried entities \`sync: false\` and read them via ` +
          `search + by-id instead of replicating the whole table.`,
      );
    } else {
      for (const local of this.store.list(entity)) {
        const id = (local as { id?: unknown }).id;
        if (typeof id !== "string") continue;
        if (pendingKeys.has(`${entity}/${id}`)) continue;
        if (!serverIds.has(id)) removalIds.push(id);
      }
    }
    if (upserts.length === 0 && removalIds.length === 0) return;
    // Route through the apply queue so a reconcile batch can't
    // interleave with a fresher WS/pull change event mid-apply.
    await this.enqueueReconcile(entity, upserts, removalIds, tombstoneSeq);
  }

  private async dropEntity(
    entity: string,
    tombstoneSeq: number,
  ): Promise<void> {
    const locals = this.store.list(entity);
    let removed = false;
    for (const local of locals) {
      const id = (local as { id?: unknown }).id;
      if (typeof id !== "string") continue;
      if (this.store.reconcileRemove(entity, id, tombstoneSeq)) {
        removed = true;
        if (this.persistence) {
          try {
            await this.persistence.deleteRow(entity, id);
          } catch {
            /* best-effort */
          }
        }
      }
    }
    if (removed) this.store.notify();
  }

  /**
   * Fetch `/api/auth/me` and feed the result into the SessionResolver,
   * acting on the verdict it returns (reset replica + pull on a real
   * tenant flip, notify subscribers if any field changed). Callers:
   *   - `start()` — initial load
   *   - the token-flip branch in `pull()`
   *   - `notifySessionChanged()` — app code invokes this after it mutates
   *     server session state (login, logout, `/api/auth/select-org`) so the
   *     cached session + React subscribers update immediately instead of
   *     waiting for the next pull/reconnect cycle.
   *
   * On tenant flip this also resets the replica — same logic as the
   * token-flip path, for the same reason (visible set changed).
   */
  async refreshResolvedSession(): Promise<void> {
    // Followers don't fetch /api/auth/me — the leader does and
    // broadcasts the result, which `handleMultiTabMessage` routes
    // into the resolver.
    if (!this.isMultiTabLeader) return;
    const next = await this.fetchSessionBootstrap();
    if (next === null) return;
    await this.applySessionTransition(next, /* broadcast */ true);
  }

  /**
   * Pure HTTP fetch of /api/auth/me → ResolvedSession. Unlike
   * `refreshResolvedSession`, this does NOT gate on `isMultiTabLeader`
   * — bootstrap callers in `start()` fire this in PARALLEL with the
   * multi-tab election to overlap two independent latency windows
   * (election ~250ms || auth/me ~60ms). At that point no other tabs'
   * messages have been observed yet, so there's no broadcast-policy
   * violation; the caller is responsible for discarding the result
   * if it lost the election.
   *
   * Returns null on HTTP error / network failure / parse error — the
   * caller's next pull cycle (or the WS `session-changed` envelope)
   * will retry. Errors must not abort bootstrap.
   */
  private async fetchSessionBootstrap(): Promise<ResolvedSession | null> {
    try {
      const res = await this.rawFetch("/api/auth/me");
      if (!res.ok) return null;
      const raw = (await res.json()) as {
        user_id?: string | null;
        tenant_id?: string | null;
        is_admin?: boolean;
        roles?: string[];
      };
      return {
        userId: raw.user_id ?? null,
        tenantId: raw.tenant_id ?? null,
        isAdmin: raw.is_admin ?? false,
        roles: raw.roles ?? [],
      };
    } catch {
      // Swallow — /api/auth/me errors are transient and the next pull
      // will retry. Don't take down the sync loop for this.
      return null;
    }
  }

  /**
   * Apply a freshly-observed session through the resolver and act on
   * the verdict. Serialized via `sessionChain` so concurrent triggers
   * (refreshResolvedSession from app code + multi-tab `session`
   * broadcast + WS `session-changed` envelope) run in arrival order
   * and the latest tenant wins — without this, two interleaved
   * inspect-then-commit pairs could commit an older session AFTER
   * the newer one.
   */
  private applySessionTransition(
    next: ResolvedSession,
    broadcast: boolean,
  ): Promise<void> {
    const prev = this.sessionChain;
    // Swallow errors when storing back to the chain so a single
    // thrown transition doesn't poison the FIFO for everyone after.
    // Callers receive the swallowed chain — they shouldn't see (or
    // need to handle) errors from a session refresh.
    this.sessionChain = prev.then(async () => {
      // Defer the null→X / X→Y / first-resolution distinction to the
      // resolver, but DON'T commit the new resolved session until
      // after the engine has finished acting on the verdict — that
      // closes the brief window where useSession would report the
      // new tenant while useQuery still has the old tenant's rows.
      const verdict = this.session.inspectSession(next);
      if (verdict.tenantChanged) {
        if (verdict.replicaInvalidated) {
          // Route reset through the public (queued) method so the
          // wipe serializes against in-flight pulls / WS-event
          // applies / pushes. sessionChain serializes session
          // transitions but NOT the apply queue — without queuing
          // the reset, a concurrent applyChangesAsync could write
          // rows AFTER we clear the store, leaving stale data under
          // the new identity. Identity flipped → wipe the outgoing
          // identity's pending offline writes too.
          await this.resetReplica({ wipeMutations: true });
        }
        if (this.isMultiTabLeader) {
          // Only the leader pulls — followers receive subsequent
          // applied broadcasts that close the catch-up window.
          await this.pull();
        }
      }
      this.session.commitObservation(next);
      if (verdict.identityChanged) {
        this.store.notify();
      }
      if (broadcast && this.isMultiTabLeader) {
        this.broadcastToTabs({ type: "session", resolved: next });
      }
    }).catch(() => {});
    return this.sessionChain;
  }

  private async rawFetch(path: string): Promise<Response> {
    const headers: Record<string, string> = {};
    const token = this.currentToken();
    if (token) headers["Authorization"] = `Bearer ${token}`;
    // `credentials: "include"` so cookie-auth apps (Yapless and
    // anyone else relying on the `<app>_session` cookie pylon sets
    // at login) actually authenticate on /api/auth/me. Without it
    // `refreshResolvedSession` returns 401 → tenantNow stays the
    // same → `resetReplica` never fires on /api/auth/select-org
    // → the local store keeps every previous tenant's rows in
    // cache and `db.useQuery` returns stale data after a switch.
    return fetch(`${this.config.baseUrl}${path}`, {
      headers,
      credentials: "include",
    });
  }

  /**
   * Public alias for `refreshResolvedSession`. Almost never needed by
   * app code today — the server pushes a `session-changed` envelope
   * over WS whenever the session is mutated (select-org, clear-org,
   * session revoke, even from other tabs / admin tools / server
   * actions), and the engine's WS handler refreshes automatically.
   *
   * Kept as an escape hatch for the rare case where you mutated the
   * session via a path that doesn't go through the framework's auth
   * surface (e.g. directly writing to the SessionStore from a Rust
   * plugin that bypassed `notify_session_changed`).
   *
   * The `selectOrg` / `clearOrg` / `signOut` helpers below remain as
   * convenience wrappers that combine the HTTP call with an immediate
   * local refresh — useful when the same tab needs the new state
   * before the WS round-trip lands.
   */
  notifySessionChanged(): Promise<void> {
    return this.refreshResolvedSession();
  }

  /**
   * Drop a row from the local replica because the server signaled
   * that the current subscriber's read policy was revoked for it.
   *
   * `tombstoneSeq` is the server's high-water seq at the time of
   * revocation (from the envelope). Stale in-flight WS frames with
   * `seq <= tombstoneSeq` are filtered locally; legitimate
   * re-grant + re-insert at higher seqs still land. Also fires a
   * catch-up pull on revocation so any frame with `seq >
   * tombstoneSeq` that arrives before the next legitimate event
   * gets reconciled against server truth.
   *
   * Uses `LocalStore.revokeRow` (not `reconcileRemove`) so the
   * tombstone is recorded even for CRDT-only consumers whose row
   * was never materialized into `tables`.
   *
   * Also notifies row-eviction listeners so external row-bound
   * resources (LoroDoc registries, etc.) can unmount.
   */
  private handleRowRevocation(
    entity: string,
    rowId: string,
    tombstoneSeq: number,
  ): void {
    const removed = this.store.revokeRow(entity, rowId, tombstoneSeq);
    if (removed) {
      if (this.persistence) {
        void this.persistence.deleteRow(entity, rowId).catch(() => {
          /* best-effort */
        });
      }
      this.store.notify();
    }
    for (const listener of this.rowEvictionListeners) {
      listener(entity, rowId);
    }
    // Fire a catch-up pull so any in-flight frame with seq above
    // the revocation tombstone is resolved against server truth.
    // Fire-and-forget — pull is internally serialized so concurrent
    // triggers coalesce.
    void this.pull();
  }

  /**
   * Register a listener invoked when the server signals a per-
   * subscriber row revocation. Used by `@pylonsync/loro` to evict
   * the LoroDoc registry entry for the row so collaborative doc
   * handles unmount cleanly. Returns an unsubscribe function.
   */
  addRowEvictionListener(
    listener: (entity: string, rowId: string) => void,
  ): () => void {
    this.rowEvictionListeners.add(listener);
    return () => {
      this.rowEvictionListeners.delete(listener);
    };
  }

  /**
   * Switch the caller's active tenant (organization) and refresh the
   * resolved session in one shot. Membership is verified server-side
   * (POST /api/auth/select-org throws 403 if the user isn't a member
   * of the target org), and the engine's local replica resets so
   * `db.useQuery` stops returning the previous tenant's rows.
   *
   * Throws on any non-2xx response. The error carries the
   * server-issued JSON error body when available, so callers can
   * branch on `err.code === "NOT_A_MEMBER"` etc.
   */
  async selectOrg(orgId: string): Promise<void> {
    await this.authMutate("/api/auth/select-org", { orgId });
    await this.refreshResolvedSession();
  }

  /**
   * Drop the caller's active tenant — back to the "no active org"
   * state typical of a login-lobby route. Refreshes the resolved
   * session so React subscribers re-render with `tenantId: null`.
   */
  async clearOrg(): Promise<void> {
    await this.authMutate("/api/auth/select-org", { orgId: null });
    await this.refreshResolvedSession();
  }

  /**
   * Revoke the current session server-side (DELETE /api/auth/session)
   * and refresh — leaves the caller anonymous. Local sync stops on
   * the next pull cycle; replica content stays in IndexedDB so a
   * subsequent sign-in as the same user is instant.
   */
  async signOut(): Promise<void> {
    await this.authMutate("/api/auth/session", undefined, "DELETE");
    await this.refreshResolvedSession();
  }

  /** Shared transport for the auth helpers above. Same bearer/cookie
   *  policy as `request()` — keeps the auth flows on the same
   *  authentication footing as data sync. */
  private async authMutate(
    path: string,
    body?: unknown,
    method = "POST",
  ): Promise<unknown> {
    const headers: Record<string, string> = {};
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const token =
      this.config.token ??
      this.storage.get(this.tokenStorageKey()) ??
      undefined;
    if (token) headers["Authorization"] = `Bearer ${token}`;
    const res = await fetch(`${this.config.baseUrl}${path}`, {
      method,
      headers,
      credentials: "include",
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    let parsed: unknown = null;
    if (text) {
      try {
        parsed = JSON.parse(text);
      } catch {
        // Server returned non-JSON (HTML error page from a proxy,
        // empty 204, etc.) — fall through; the !res.ok branch will
        // synthesise a useful Error from the status.
      }
    }
    if (!res.ok) {
      const err = new Error(
        (parsed as { error?: { message?: string } } | null)?.error?.message ??
          `${method} ${path} failed: ${res.status}`,
      ) as Error & { status?: number; code?: string };
      err.status = res.status;
      const code = (parsed as { error?: { code?: string } } | null)?.error
        ?.code;
      if (code) err.code = code;
      throw err;
    }
    return parsed;
  }

  /** Push pending mutations to the server. Coalesces concurrent callers
   *  via the op queue's keyed dedupe — a slow push can't be restarted
   *  by the poll timer or a user mutation, which would resend the same
   *  batch (the mutation `op_id` keeps that safe at the protocol level,
   *  but shipping the same batch twice is still wasted bandwidth). Also
   *  serializes against pull / reconcile / resetReplica so a push can't
   *  observe a half-reset cursor or a mid-reconcile replica. */
  async push(): Promise<void> {
    return this.opQueue.enqueue("push", () => this.pushInner());
  }

  private async pushInner(): Promise<void> {
    const pending = this.mutations.pending();
    if (pending.length === 0) return;

    // Multi-tab follower: we don't own the network. Forward the
    // pending batch to the leader and let it push. The leader
    // broadcasts `mutations-acked` when the server confirms; that
    // path clears our queue. Note we don't clear locally here — if
    // the leader dies before pushing, on promotion we still have
    // the queue and can ship it ourselves.
    if (!this.isMultiTabLeader) {
      this.broadcastToTabs({ type: "mutations", ops: pending });
      return;
    }

    try {
      const resp = await this.request<PushResponse>("POST", "/api/sync/push", {
        changes: pending.map((m) => m.change),
        client_id: this.clientId,
      });
      // The request reached the server and returned a response — clear
      // the transient-failure backoff counter (success or per-op
      // rejections both mean "we're online and the server answered").
      this.pushFailureCount = 0;

      // Per-op `results` mapping: match by op_id when present, fall
      // back to positional. Invariant: a partial-failure batch lands
      // the correct status on each mutation by id, never by position.
      // Test: `push_partial_failure_maps_results_by_op_id`.
      let maxAppliedSeq = 0;
      let hasInFlightDedupe = false;
      if (Array.isArray(resp.results)) {
        const byOpId = new Map<string, PushOpResult>();
        for (const r of resp.results) {
          if (r.op_id) byOpId.set(r.op_id, r);
        }
        for (let i = 0; i < pending.length; i++) {
          const m = pending[i];
          const r =
            (m.change.op_id ? byOpId.get(m.change.op_id) : undefined) ??
            resp.results[i];
          if (!r) continue;
          // applied: first-time commit at r.seq.
          // replayed: same op_id arrived again after a confirmed apply;
          // r.seq is the original write's seq. Both are terminal-success
          // from the client's perspective.
          // deduped: legacy server response — treat as replayed.
          if (r.status === "applied" || r.status === "replayed" || r.status === "deduped") {
            this.mutations.markApplied(m.id);
            if (typeof r.seq === "number" && r.seq > maxAppliedSeq) {
              maxAppliedSeq = r.seq;
            }
          } else if (r.status === "pending") {
            // A concurrent push carrying this op_id is still in
            // flight on the server. Keep the mutation queued; a
            // later push() will retry. The client must NOT mark
            // applied here — the in-flight writer might fail and
            // forget the claim, leaving the row un-committed.
            hasInFlightDedupe = true;
          } else if (r.status === "error") {
            const msg =
              typeof r.error === "string"
                ? r.error
                : r.error?.message ?? "unknown";
            this.failPushedMutation(m, msg);
          }
        }
      } else {
        // Legacy server response (pre-0.3.188): count-based mapping.
        // Buggy on partial failures but the best we can do without
        // the per-op envelope. Guard `resp.errors` — older test
        // mocks omit the field entirely; pre-0.3.224 the `[]` index
        // threw and was swallowed by the bare catch below, which
        // silently dropped the success path for legacy responses.
        const applied = typeof resp.applied === "number" ? resp.applied : 0;
        const errors = Array.isArray(resp.errors) ? resp.errors : [];
        for (let i = 0; i < pending.length; i++) {
          if (i < applied) {
            this.mutations.markApplied(pending[i].id);
          } else if (errors[i - applied]) {
            this.failPushedMutation(pending[i], errors[i - applied]);
          }
        }
      }

      // Broadcast per-op outcomes BEFORE clearing locally so followers
      // can update their queue status. Filter strictly by current
      // status — pending[i] is the LIVE PendingMutation that markApplied
      // / markFailed just mutated, so `m.status` tells us exactly what
      // happened. Ops that stayed "pending" (server-side in-flight
      // dedupe) get neither — the leader will retry, and a later push
      // will broadcast a real ack.
      const ackedOpIds: string[] = [];
      const failedOps: { opId: string; error: string }[] = [];
      for (const m of pending) {
        const opId = m.change.op_id;
        if (typeof opId !== "string") continue;
        if (m.status === "applied") {
          ackedOpIds.push(opId);
        } else if (m.status === "failed") {
          failedOps.push({ opId, error: m.error ?? "unknown" });
        }
      }
      if (ackedOpIds.length > 0) {
        this.broadcastToTabs({ type: "mutations-acked", opIds: ackedOpIds });
      }
      if (failedOps.length > 0) {
        this.broadcastToTabs({ type: "mutations-failed", ops: failedOps });
      }

      this.mutations.clear();

      // Catch-up pull: if the server confirmed an apply at a seq
      // ahead of our local cursor, request the delta now so the
      // local replica picks up server-side defaults / plugin fields
      // / linked rows without waiting for the WS broadcast (the WS
      // event is the happy path; this is the fallback for
      // dropped/delayed frames).
      if (maxAppliedSeq > this.cursor.last_seq) {
        // Fire-and-forget — pull() is internally serialized via
        // inFlightPull so concurrent triggers from WS + this branch
        // coalesce.
        void this.pull();
      }
      // If any op came back with status="pending" (a concurrent push
      // is still in flight on the server for the same op_id), schedule
      // a retry shortly. The first writer will either Commit (and
      // we'll get the canonical seq on next push, or pick it up via
      // WS rebroadcast) or Fail (the entry is forgotten, our retry
      // takes the Proceed slot). 250ms is short enough that user
      // perception doesn't notice, long enough to not hot-loop.
      if (hasInFlightDedupe) {
        setTimeout(() => {
          void this.push();
        }, 250);
      }
    } catch (err) {
      // Whole-request failure. CRITICAL distinction:
      //
      //  - TRANSIENT (offline / network drop / 5xx / 429 / 401 / 408):
      //    the server never durably rejected the write. We MUST keep
      //    the mutations `pending` and the optimistic ghost intact, and
      //    retry with backoff. Marking them failed + rolling back here
      //    is what broke offline support — an offline insert vanished
      //    from the UI and was never re-sent (it became `failed`, and
      //    pushInner only ships `pending`). A network `fetch` throw has
      //    NO `.status`, so it lands here as transient. op_id makes the
      //    eventual retry idempotent even if the server HAD committed.
      //
      //  - PERMANENT (400/403/404/409/422): a client error that won't
      //    change on retry (malformed batch, forbidden, gone). Fail +
      //    roll back the optimistic ghost + surface mutations-failed.
      const msg = err instanceof Error ? err.message : String(err);
      const status = (err as { status?: number })?.status;
      if (isPermanentPushError(status)) {
        const failedOps: { opId: string; error: string }[] = [];
        for (const m of pending) {
          this.failPushedMutation(m, msg);
          const opId = m.change.op_id;
          if (typeof opId === "string") {
            failedOps.push({ opId, error: msg });
          }
        }
        if (failedOps.length > 0) {
          this.broadcastToTabs({ type: "mutations-failed", ops: failedOps });
        }
        this.mutations.clear();
        // eslint-disable-next-line no-console
        console.warn(`[sync] /api/sync/push rejected (status ${status}):`, msg);
      } else {
        // Transient: leave the queue + ghosts alone, retry with bounded
        // exponential backoff. Resets on the next response (success or
        // per-op rejection). A 429 also pushes the WS reconnect out so a
        // rate-limited push doesn't drive a tight loop.
        if (status === 429) this.transport?.bumpReconnect(3);
        const attempt = this.pushFailureCount;
        this.pushFailureCount += 1;
        const delayMs = Math.min(30_000, 1000 * 2 ** Math.min(attempt, 5));
        // eslint-disable-next-line no-console
        console.warn(
          `[sync] /api/sync/push transient failure (status ${status ?? "offline"}); keeping ${pending.length} mutation(s) pending, retrying in ${delayMs}ms`,
        );
        setTimeout(() => {
          void this.push();
        }, delayMs);
      }
    }
  }

  /**
   * Mark a pending mutation as failed AND undo its optimistic ghost
   * in the local replica. Without the rollback step, a server-
   * rejected insert leaves a ghost row that survives indefinitely
   * (reconcile skips rows with pending/failed mutations to avoid
   * sweeping the user's in-flight edit). The exact failure mode is
   * "send a message, the server says no, refresh — the ghost is
   * still there until you find the failed-state UI."
   *
   * Updates can't be rolled back without a pre-update snapshot
   * (not captured today); the user-visible ghost-update sticks
   * until the next reconcile observes the canonical row. Deletes
   * leave a tombstone that should also be cleared, but the current
   * `LocalStore` API doesn't expose the un-tombstone path — flagged
   * for follow-up. Inserts are the dominant case (chat send is an
   * insert; collaborative-edit is an update with a separate CRDT
   * channel) so insert-only rollback is the right shape to ship now.
   */
  private failPushedMutation(m: PendingMutation, error: string): void {
    const { entity, row_id, kind } = m.change;
    if (kind === "insert") {
      // No tombstone — a future legitimate insert of this id must work.
      this.store.rollbackOptimisticInsert(entity, row_id);
    } else if (kind === "update" || kind === "delete") {
      // Restore the captured pre-mutation row (update: prior field
      // values; delete: bring it back AND clear the optimistic tombstone
      // fence). `prevRow === null` means the row didn't exist pre-mutation
      // → remove + un-fence. `prevRow === undefined` means THIS engine
      // never captured a snapshot — i.e. the optimistic change wasn't
      // applied to this store (a forwarded op whose prevRow didn't
      // thread). Touching the store then would delete a canonical row we
      // still hold, so leave it untouched and let pull/reconcile
      // reconverge. The `!== undefined` guard distinguishes the two.
      if (m.prevRow !== undefined) {
        this.store.restoreRow(entity, row_id, m.prevRow);
      }
    }
    this.mutations.markFailed(m.id, error);
  }

  /** Insert a row with optimistic local update.
   *
   *  Invariant: the optimistic ghost and the canonical server row
   *  share a single id. The client mints a Pylon-shaped id, threads
   *  it through the data payload, and the server honors it on the
   *  canonical insert. Test:
   *  `insert_optimistic_ghost_and_server_row_share_id`. */
  async insert(entity: string, data: Row): Promise<string> {
    const id = generateId();
    const dataWithId = { ...data, id };
    this.store.optimisticInsertWithId(entity, id, dataWithId);
    this.mutations.add({
      entity,
      row_id: id,
      kind: "insert",
      data: dataWithId,
    });
    await this.push();
    return id;
  }

  /** Update a row with optimistic local update. */
  async update(entity: string, id: string, data: Partial<Row>): Promise<void> {
    // Snapshot the pre-update row BEFORE applying the optimistic merge so
    // a rejected push can restore the exact prior value (see
    // failPushedMutation). Clone — the live row is mutated in place.
    const before = this.store.get(entity, id);
    const prev = before ? { ...before } : null;
    this.store.optimisticUpdate(entity, id, data);
    this.mutations.add(
      {
        entity,
        row_id: id,
        kind: "update",
        data: data as Row,
      },
      prev,
    );
    await this.push();
  }

  /** Delete a row with optimistic local update. */
  async delete(entity: string, id: string): Promise<void> {
    // Snapshot the row before removing it so a rejected delete can bring
    // it back (and clear the optimistic tombstone).
    const before = this.store.get(entity, id);
    const prev = before ? { ...before } : null;
    this.store.optimisticDelete(entity, id);
    this.mutations.add(
      {
        entity,
        row_id: id,
        kind: "delete",
      },
      prev,
    );
    await this.push();
  }

  // -----------------------------------------------------------------------
  // Infinite scroll / cursor pagination
  // -----------------------------------------------------------------------

  /** Load a page of data from an entity with cursor-based pagination. */
  async loadPage(
    entity: string,
    options?: { limit?: number; offset?: number; order?: Record<string, "asc" | "desc"> }
  ): Promise<{ data: Row[]; total: number; hasMore: boolean }> {
    const limit = options?.limit ?? 20;
    const offset = options?.offset ?? 0;

    const filter: Record<string, unknown> = {
      $limit: limit,
      $offset: offset,
    };
    if (options?.order) {
      filter.$order = options.order;
    }

    const resp = await this.request<Row[]>(
      "POST",
      `/api/query/${entity}`,
      filter
    );

    const data = Array.isArray(resp) ? resp : [];
    return {
      data,
      total: data.length, // Server doesn't return total in filtered query
      hasMore: data.length === limit,
    };
  }

  /**
   * Create an infinite query that appends pages.
   * Returns an object with loadMore() and the current accumulated data.
   */
  createInfiniteQuery(entity: string, options?: { pageSize?: number; order?: Record<string, "asc" | "desc"> }) {
    const pageSize = options?.pageSize ?? 20;
    let allRows: Row[] = [];
    let offset = 0;
    let hasMore = true;
    let loading = false;

    const listeners = new Set<() => void>();

    const notify = () => {
      for (const fn of listeners) fn();
    };

    return {
      /** Load the next page. */
      loadMore: async () => {
        if (!hasMore || loading) return;
        loading = true;
        try {
          const page = await this.loadPage(entity, { limit: pageSize, offset, order: options?.order });
          allRows = [...allRows, ...page.data];
          offset += page.data.length;
          hasMore = page.hasMore;
          notify();
        } finally {
          loading = false;
        }
      },
      /** Get current accumulated rows. */
      get data() { return allRows; },
      /** Whether more pages are available. */
      get hasMore() { return hasMore; },
      /** Whether currently loading. */
      get loading() { return loading; },
      /** Subscribe to changes. */
      subscribe: (fn: () => void) => {
        listeners.add(fn);
        return () => listeners.delete(fn);
      },
      /** Reset and start over. */
      reset: () => {
        allRows = [];
        offset = 0;
        hasMore = true;
      },
    };
  }

  /** Get the current cursor position. */
  getCursor(): SyncCursor {
    return { ...this.cursor };
  }

  /** Whether the real-time transport is currently open. True for an
   *  open WebSocket / EventSource and for a running poll loop; false
   *  for a follower tab (no transport) or before start() / after stop(). */
  get connected(): boolean {
    return this.transport?.isOpen() ?? false;
  }

  // -----------------------------------------------------------------------
  // Presence
  // -----------------------------------------------------------------------

  /** Set this client's presence data and broadcast it. */
  setPresence(data: Record<string, unknown>): void {
    this.presenceData = data;
    this.sendWs({
      type: "presence",
      event: "update",
      data: this.presenceData,
    });
  }

  /** Send a topic message to all connected clients. */
  publishTopic(topic: string, data: unknown): void {
    this.sendWs({
      type: "topic",
      topic,
      data,
    });
  }

  /**
   * Subscribe this client to binary CRDT updates for one row. Refcounted
   * so two `useLoroDoc` consumers on the same `(entity, rowId)` don't
   * unsubscribe each other on unmount — only the last `unsubscribeCrdt`
   * call ships the unsubscribe message to the server.
   *
   * The first subscriber for a row sends the `crdt-subscribe` over WS,
   * which prompts the server to ship the current snapshot back as a
   * binary frame so the new tab converges to the latest state.
   *
   * Idempotent at the WS level: re-calling for the same row with no
   * intervening unsubscribe just bumps the refcount.
   */
  subscribeCrdt(entity: string, rowId: string): void {
    this.subscriptions.subscribeCrdt(entity, rowId);
  }

  /**
   * Decrement the refcount for a row. When it hits zero we ship a
   * `crdt-unsubscribe` to the server and forget the row, so a future
   * reconnect won't try to resubscribe.
   *
   * Calling `unsubscribeCrdt` more times than `subscribeCrdt` is a
   * no-op rather than an error — keeps React's StrictMode double-
   * invocation in dev from over-decrementing past zero.
   */
  unsubscribeCrdt(entity: string, rowId: string): void {
    this.subscriptions.unsubscribeCrdt(entity, rowId);
  }

  // -----------------------------------------------------------------------
  // Reactive query subscriptions
  //
  // Convex-shaped: the client mounts `useReactiveQuery(fnName, args)`,
  // the server runs the handler with dep tracking, registers the sub,
  // and pushes the initial result. Whenever a future mutation touches
  // anything in the dep set, the server re-runs the handler and pushes
  // the new result. The handler always re-runs under the subscriber's
  // original auth context — not the mutating user's — so policy gates
  // applied at first run apply on every re-run.
  // -----------------------------------------------------------------------

  /**
   * Register a reactive query subscription. The caller-minted `sub_id`
   * is used by the React hook to dispatch result/error pushes to the
   * right component. Returns nothing — push handling is async via the
   * registered handler.
   *
   * Idempotent: re-calling with the same `sub_id` replaces the prior
   * handler + spec. Useful when args change and the hook re-registers.
   *
   * The actual subscribe message goes over the WS — works only when
   * the socket is open. When called before the WS opens (initial
   * mount during start()), the spec is still recorded and gets sent
   * on `ws.onopen`'s re-registration sweep.
   */
  subscribeReactive(
    sub_id: string,
    fn_name: string,
    args: unknown,
    handler: (msg: ReactiveMessage) => void,
  ): void {
    this.subscriptions.subscribeReactive(sub_id, fn_name, args, handler);
  }

  /** Tear down a reactive subscription. Sends the unsubscribe to the
   *  server and clears local state. No-op for unknown sub_ids — React
   *  StrictMode double-unmount won't error. */
  unsubscribeReactive(sub_id: string): void {
    this.subscriptions.unsubscribeReactive(sub_id);
  }

  /** Send a JSON message via the active transport. No-op when no
   *  transport exists (follower tab) or the transport doesn't support
   *  uplink (SSE, polling). Subscribe / presence / topic / ping frames
   *  all route here. */
  private sendWs(msg: unknown): void {
    this.transport?.send(msg);
  }

  // -----------------------------------------------------------------------
  // Room presence subscriptions
  //
  // Replaces the SDK's per-component `setInterval(GET /api/rooms/<room>,
  // 5s)` polling loop. Wire shape (server v0.3.214+):
  //
  //   client → server:
  //     { type: "room-subscribe",   room: "channel:foo" }
  //     { type: "room-unsubscribe", room: "channel:foo" }
  //   server → client:
  //     { type: "room-snapshot", room, members: [...] }
  //     { type: "room-update",   room, action: "join"|"leave"|"presence"|"broadcast",
  //                                          member?, data? }
  //     { type: "error", code: "NOT_IN_ROOM", room }   // server gate
  //
  // The actual `POST /api/rooms/join` still happens over HTTP (it carries
  // initial presence + identity, returns the snapshot). This API only
  // moves the "stay subscribed to membership deltas" loop off polling
  // and onto the WS push channel. setPresence + broadcast also stay on
  // HTTP for now — the WS-RPC envelope for those lands in v0.3.217.
  // -----------------------------------------------------------------------

  /**
   * Subscribe to a room's membership over WebSocket. The callback fires
   * whenever the room's `members` list OR `error` state changes — read
   * the current snapshot via `getRoomMembers(roomId)` and the latest
   * error via `getRoomError(roomId)` inside the callback.
   *
   * Refcounted: multiple subscribers for the same `roomId` share one
   * `room-subscribe` on the wire. Returns an unsubscribe function;
   * the last unsubscribe ships `room-unsubscribe`.
   *
   * Follower tabs forward the register / unregister over the multi-tab
   * channel so the leader's WS carries one subscribe per room across
   * the whole origin. Inbound snapshot / update / error envelopes land
   * on the leader's WS and the leader fans them out cross-tab so each
   * follower's local registry routes to its own subscribers.
   *
   * Idempotent w.r.t. wire frames: a re-subscribe with no intervening
   * full unsubscribe doesn't re-send `room-subscribe`. ServerSubscriptions-
   * style replay on reconnect is built in — the registry resends
   * `room-subscribe` for every active room on WS reopen.
   */
  subscribeRoom(roomId: string, callback: RoomSubscriber): () => void {
    if (this.isMultiTabLeader) {
      // Leader path: register against the local registry which owns
      // the wire. If a follower had already forwarded this room, the
      // WS sub is already alive — `register()` short-circuits the
      // re-send via the registry's first-add gate.
      return this.rooms.register(roomId, callback);
    }
    // Follower path: register locally for callback routing, then ask
    // the leader to subscribe on our behalf (only on first local
    // subscriber for the room — multiple mounts in the same follower
    // tab share one `room-sub-register`, mirroring the leader-side
    // ServerSubscriptions first-add gate). The leader echoes snapshots
    // / updates / errors back over the broadcast channel and our local
    // registry's notify() fires our callback.
    const hadRoomBefore = this.rooms.has(roomId);
    const unsubscribe = this.rooms.register(roomId, callback);
    if (!hadRoomBefore) {
      this.broadcastToTabs({
        type: "room-sub-register",
        room: roomId,
      });
    }
    return () => {
      unsubscribe();
      // If this was the LAST subscriber on this tab for the room,
      // tell the leader so it can drop us from the forwarder set.
      if (!this.rooms.has(roomId)) {
        this.broadcastToTabs({
          type: "room-sub-unregister",
          room: roomId,
        });
      }
    };
  }

  /**
   * Subscribe to BROADCAST MESSAGES relayed through a room (the
   * payloads sent via `POST /api/rooms/broadcast` / `useRoom`'s
   * `broadcast()`). Same refcounted wire-subscription and leader /
   * follower routing as `subscribeRoom` — the two channels share one
   * `room-subscribe` per room per origin.
   *
   * The callback receives `{ topic, payload, from }` where `from` is
   * the server-stamped sender user id (own broadcasts echo back —
   * filter on `from` if unwanted). Returns an unsubscribe function.
   */
  subscribeRoomMessages(
    roomId: string,
    callback: (message: import("./room-subscriptions").RoomMessage) => void,
  ): () => void {
    if (this.isMultiTabLeader) {
      return this.rooms.registerMessages(roomId, callback);
    }
    // Follower path mirrors subscribeRoom: register locally for
    // routing, ask the leader to hold the wire sub on first add.
    const hadRoomBefore = this.rooms.has(roomId);
    const unsubscribe = this.rooms.registerMessages(roomId, callback);
    if (!hadRoomBefore) {
      this.broadcastToTabs({ type: "room-sub-register", room: roomId });
    }
    return () => {
      unsubscribe();
      if (!this.rooms.has(roomId)) {
        this.broadcastToTabs({ type: "room-sub-unregister", room: roomId });
      }
    };
  }

  /** Force-unsubscribe every local subscriber of a room and ship a
   *  `room-unsubscribe`. Used by the `useRoom` hook's manual `leave()`
   *  action so a deliberate exit propagates to the server immediately. */
  unsubscribeRoom(roomId: string): void {
    this.rooms.unregisterRoom(roomId);
    if (!this.isMultiTabLeader) {
      this.broadcastToTabs({
        type: "room-sub-unregister",
        room: roomId,
      });
    }
  }

  /** Read the current cached members snapshot for `roomId`. Returns
   *  `null` when no snapshot has landed yet (distinct from `[]` for
   *  an empty room). */
  getRoomMembers(roomId: string): RoomMember[] | null {
    return this.rooms?.members(roomId) ?? null;
  }

  /** Read the latest error for `roomId` (e.g. NOT_IN_ROOM). null when
   *  none. */
  getRoomError(roomId: string): RoomError | null {
    return this.rooms?.error(roomId) ?? null;
  }

  /** Active transport kind. Used by the `useRoom` hook to decide
   *  between WS push and HTTP polling fallback — only the WS transport
   *  supports the room-subscribe push protocol; SSE and polling fall
   *  back to the legacy 5s GET /api/rooms/<room>. */
  getActiveTransportType(): TransportType {
    return this.transportKind();
  }

  /** True when the active transport is a WebSocket AND the socket is
   *  currently open. The `useRoom` hook gates its WS-push path on this
   *  — when false, fall back to polling. */
  isWebSocketConnected(): boolean {
    return this.transportKind() === "websocket" && this.connected;
  }

  /** Resolved transport kind, with the websocket default applied. */
  private transportKind(): TransportKind {
    return this.config.transport ?? "websocket";
  }

  /** Build the host surface the transport calls back into. One object
   *  shared across transport lifetime — the engine fields it reads (
   *  config, transport state, callbacks) are stable references. */
  private transportHost(): TransportHost {
    return {
      baseUrl: this.config.baseUrl,
      wsUrl: this.config.wsUrl,
      pingIntervalMs: this.config.pingIntervalMs,
      reconnectDelayMs: this.config.reconnectDelay,
      pollIntervalMs: this.config.pollInterval,
      getToken: () => this.currentToken() ?? undefined,
      isLeader: () => this.isMultiTabLeader,
      isRunning: () => this.running,
      onChangeEvent: (ev) => {
        void this.enqueueApply([ev]);
      },
      onJsonMessage: (msg) => this.dispatchInboundMessage(msg),
      onBinaryFrame: (bytes) => this.dispatchBinaryFrame(bytes),
      onConnected: () => {
        // Re-send every active server-subscription (CRDT rows,
        // reactive queries, future kinds) across the new socket. The
        // server purges per-client subscription state on disconnect,
        // so without this resync the subscriber's first event would
        // never arrive.
        this.serverSubs.replay();
        // Room subscriptions live in their own registry (not under
        // serverSubs) because they carry per-room state (members
        // snapshot, error). Replay them on the same beat so the
        // server starts pushing room-snapshot/room-update again for
        // every room the user is still subscribed to.
        this.rooms.replay();
        // Pull-on-open catches every event broadcast in the gap
        // between the prior pull() returning and the socket actually
        // opening. Reconcile fires after the pull since pull is the
        // cheap incremental path; reconcile is the server-truth
        // backstop for anything pull couldn't replay.
        //
        // Cold-load fast path: if pull just hydrated a full snapshot
        // from cursor=0, the snapshot already returned every row
        // visible under current policy. The reconcile pass that would
        // normally follow is pure waste — same rows, second time,
        // ~60ms × N entities. Skip it once; visibility-change and
        // reconnect-after-disconnect paths invoke reconcile() directly
        // (not gated by this flag) so the safety net still triggers.
        void this.pull().then(() => {
          // Cold-load fast-path: skip reconcile only when this WAS a
          // true cold start (no IDB cache → the pull-from-0 returned
          // every visible row, reconcile would refetch the same set).
          // A returning user whose pull happened to start from 0
          // (cursor rolled back, partial cache wipe) MUST still run
          // reconcile to catch rows deleted on the server while the
          // tab was closed — the snapshot path only returns currently-
          // visible rows, never tombstones, so ghost rows on the
          // cached side persist without the reconcile pass.
          if (this.lastPullStartedFromZero && !this._hadCachedReplica) {
            this.lastPullStartedFromZero = false;
            return;
          }
          this.lastPullStartedFromZero = false;
          return this.reconcile();
        });
      },
      onDisconnected: () => {
        /* Engine has no work on disconnect — the transport's own
         *  reconnect loop drives the recovery. The connection-status
         *  flip already happened inside the transport. */
      },
      setStatus: (s) => this.setConnectionStatus(s),
      performPollTick: async () => {
        await this.push().then(() => this.pull());
      },
      performReconnectPull: async () => {
        // Wrapped in try so a transient pull failure doesn't kill the
        // reconnect chain; the next attempt will retry.
        try {
          await this.pull();
        } catch {
          /* best-effort */
        }
      },
    };
  }

  /** Dispatch a typed JSON envelope inbound from the transport. The
   *  transport already filtered out ChangeEvents (those go through
   *  onChangeEvent → enqueueApply). Anything else lands here. */
  private dispatchInboundMessage(msg: Record<string, unknown>): void {
    // Presence event.
    if (msg.type === "presence") {
      this.store.notify();
      return;
    }

    // Server-driven revocation: a subscriber whose read policy was
    // revoked mid-session for a specific row. Drop the row from the
    // local replica at the current cursor seq so the tombstone
    // supersedes any racing late-arriving WS update for the same
    // row, and notify any LoroDoc subscriber (registered via
    // `addRowEvictionListener`) so collaborative doc handles unmount
    // cleanly.
    //
    // Distinct from a regular Delete change event because this
    // envelope has no global seq — the row's underlying data hasn't
    // been deleted, only the recipient's visibility of it. Other
    // subscribers (with matching policy) keep their row intact.
    if (
      msg.type === "row-revoked" &&
      typeof msg.entity === "string" &&
      typeof msg.row_id === "string"
    ) {
      const revokeSeq =
        typeof msg.seq === "number" && msg.seq > 0
          ? (msg.seq as number)
          : this.cursor.last_seq;
      this.handleRowRevocation(msg.entity, msg.row_id, revokeSeq);
      return;
    }

    // Session mutated server-side. Fires for select-org / clear-org
    // / session revoke — every tab connected as this user gets the
    // envelope (cross-machine too via the cluster bus). Trigger a
    // fresh /api/auth/me read which updates the cached session AND,
    // on tenant flip, resets the replica so stale rows from the
    // previous tenant disappear.
    if (msg.type === "session-changed") {
      void this.refreshResolvedSession();
      return;
    }

    // Room push: snapshot / update / error envelopes from the WS.
    // Leader receives them on its socket and routes through the local
    // registry; if any follower forwarded a sub for the room, fan the
    // envelope out cross-tab so follower-local registries pick up the
    // same state. The followers receive `room-fanout-*` envelopes
    // through the orchestrator and apply them to their own registries.
    if (msg.type === "room-snapshot" && typeof msg.room === "string") {
      const room = msg.room;
      const members = Array.isArray(msg.members)
        ? (msg.members as RoomMember[])
        : [];
      this.rooms.applySnapshot(room, members);
      if ((this.roomForwarders.get(room)?.size ?? 0) > 0) {
        this.broadcastToTabs({
          type: "room-fanout-snapshot",
          room,
          members,
        });
      }
      return;
    }
    if (msg.type === "room-update" && typeof msg.room === "string") {
      const room = msg.room;
      const action = msg.action as
        | "join"
        | "leave"
        | "presence"
        | "broadcast"
        | undefined;
      if (!action) return;
      const member = (msg.member as RoomMember | undefined) ?? undefined;
      const data = msg.data;
      this.rooms.applyUpdate(room, action, member, data);
      if ((this.roomForwarders.get(room)?.size ?? 0) > 0) {
        this.broadcastToTabs({
          type: "room-fanout-update",
          room,
          action,
          member,
          data,
        });
      }
      return;
    }
    if (
      msg.type === "error" &&
      typeof msg.room === "string" &&
      typeof msg.code === "string"
    ) {
      // Server-side gate failure on a room subscribe. Surface to the
      // hook via the registry so the React side can render an error
      // state. Server only emits this in the `room-subscribe` reject
      // path right now, but the dispatch is keyed by `code` so future
      // error codes (rate-limited, room-full, etc.) route here too.
      const room = msg.room;
      const code = msg.code === "NOT_IN_ROOM" ? "NOT_IN_ROOM" : "UNKNOWN";
      const error: RoomError = {
        code,
        message: typeof msg.message === "string" ? msg.message : undefined,
      };
      this.rooms.applyError(room, error);
      if ((this.roomForwarders.get(room)?.size ?? 0) > 0) {
        this.broadcastToTabs({
          type: "room-fanout-error",
          room,
          error,
        });
      }
      return;
    }

    // Reactive query push: the server-side ReactiveRegistry re-ran a
    // subscribed handler and the result hash changed. Route to the
    // local handler if we own the subscription AND forward to follower
    // tabs via the multi-tab channel so a follower's
    // `useReactiveQuery` handler fires too.
    if (msg.type === "reactive-result" && typeof msg.sub_id === "string") {
      const payload = { kind: "result" as const, result: msg.result };
      this.subscriptions.handleReactiveMessage(msg.sub_id, payload);
      this.broadcastToTabs({
        type: "reactive-msg",
        sub_id: msg.sub_id,
        payload,
      });
      return;
    }
    if (msg.type === "reactive-error" && typeof msg.sub_id === "string") {
      const errPayload = {
        kind: "error" as const,
        code: typeof msg.code === "string" ? msg.code : "REACTIVE_ERROR",
        message: typeof msg.message === "string" ? msg.message : "",
      };
      this.subscriptions.handleReactiveMessage(msg.sub_id, errPayload);
      this.broadcastToTabs({
        type: "reactive-msg",
        sub_id: msg.sub_id,
        payload: errPayload,
      });
      return;
    }
  }

  /** Route a binary frame to local consumers AND, when at least one
   *  follower tab forwarded a CRDT sub, mirror over the multi-tab
   *  channel so followers see Loro updates too. */
  private dispatchBinaryFrame(bytes: Uint8Array): void {
    for (const handler of this.binaryHandlers) {
      try {
        handler(bytes);
      } catch (err) {
        console.warn("[sync] binary handler threw:", err);
      }
    }
    // Forward to follower tabs ONLY when at least one follower is
    // currently forwarded for a CRDT row. The engine is binary-
    // agnostic — it can't peek inside the frame to route per-row —
    // so this is a tab-level gate: no forwarders = no broadcast.
    // Saves bandwidth in the common single-tab case.
    //
    // Trade-off: when ANY follower has forwarded a CRDT sub on ANY
    // key, we broadcast EVERY binary frame regardless of which row
    // it's for. Acceptable for now; the lever to pull if it shows
    // up in profiling is a binaryRoutes map keyed by the Loro doc
    // id parsed from the frame header.
    if (this.subscriptions.hasCrdtForwarders()) {
      this.broadcastToTabs({ type: "binary", bytes });
    }
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = {};
    if (body) headers["Content-Type"] = "application/json";
    // Prefer the token explicitly configured on the engine; fall back to
    // the conventional localStorage key that `@pylonsync/react`'s auth
    // helpers store. Without this fallback, the sync engine runs as an
    // anonymous caller and gets rate-limited into a 429 reconnect storm
    // once the anon bucket fills.
    const token =
      this.config.token ??
      this.storage.get(this.tokenStorageKey()) ??
      undefined;
    if (token) headers["Authorization"] = `Bearer ${token}`;

    // credentials: "include" so cookie-auth apps (Yapless and any other
    // app relying on the `<app>_session` cookie pylon sets at login)
    // actually authenticate on /api/sync/pull + /api/entities/<E>/cursor.
    // Without it the pull goes anonymous, every policy default-denies,
    // and the response is `{changes: []}` even when the same browser
    // session can read every row via the entity API. Reported in
    // Repro C against v0.3.131; closed in v0.3.134.
    const res = await fetch(`${this.config.baseUrl}${path}`, {
      method,
      headers,
      credentials: "include",
      body: body ? JSON.stringify(body) : undefined,
    });

    if (!res.ok) {
      // Surface the status so the caller can distinguish transient
      // (429/503) from permanent (400/404) failures — the reconnect
      // loop uses this to decide whether to back off.
      const err = new Error(`Sync request failed: ${res.status}`) as Error & {
        status?: number;
      };
      err.status = res.status;
      throw err;
    }

    return res.json() as Promise<T>;
  }
}

// ---------------------------------------------------------------------------
// SSR / Hydration types
// ---------------------------------------------------------------------------

/** Data shape for hydrating the client from server-rendered content. */
export interface HydrationData {
  /** Map of entity name -> rows fetched on the server. */
  entities: Record<string, Record<string, unknown>[]>;
  /** The sync cursor at the time of server fetch. */
  cursor?: SyncCursor;
}

/**
 * Server-side helper: fetch entities from the pylon API and return
 * hydration data that can be passed to the client's SyncEngine.hydrate().
 *
 * Use this in Next.js server components, getServerSideProps, or route handlers.
 */
export async function getServerData(
  baseUrl: string,
  entities: string[],
  options?: { token?: string }
): Promise<HydrationData> {
  const headers: Record<string, string> = {};
  if (options?.token) {
    headers["Authorization"] = `Bearer ${options.token}`;
  }

  const entityData: Record<string, Record<string, unknown>[]> = {};

  for (const entity of entities) {
    try {
      const res = await fetch(`${baseUrl}/api/entities/${entity}`, { headers });
      if (res.ok) {
        entityData[entity] = (await res.json()) as Record<string, unknown>[];
      } else {
        entityData[entity] = [];
      }
    } catch {
      entityData[entity] = [];
    }
  }

  // Get current sync cursor.
  let cursor: SyncCursor = { last_seq: 0 };
  try {
    const res = await fetch(`${baseUrl}/api/sync/pull?since=0&limit=0`, { headers });
    if (res.ok) {
      const pull = (await res.json()) as PullResponse;
      cursor = pull.cursor;
    }
  } catch {
    // Use beginning cursor.
  }

  return { entities: entityData, cursor };
}

/**
 * Stable equality check for reconciler diffs. Keys are sorted so
 * `{a:1,b:2}` and `{b:2,a:1}` compare equal — without that, every
 * reconcile pass would think every row had changed (insertion order
 * varies by mutation path on the server). Recursive on objects only;
 * arrays and primitives use their natural shape.
 */
function rowsDiffer(a: Row, b: Row): boolean {
  return stableStringify(a) !== stableStringify(b);
}

/**
 * Is a whole-request push failure PERMANENT (the write was durably
 * rejected and won't succeed on retry) vs TRANSIENT (offline / server
 * hiccup / rate limit — retry will eventually land)?
 *
 *  - `undefined` status = a `fetch` network throw (offline, DNS, CORS,
 *    connection reset) → transient.
 *  - 400/403/404/409/422 = client errors that are stable across retries
 *    (malformed batch, forbidden, gone, conflict, unprocessable) →
 *    permanent.
 *  - everything else (5xx, 429 rate-limit, 408 timeout, 401 needs
 *    re-auth, 502/503/504) → transient: keep the mutation queued and
 *    retry. Per-op policy rejections do NOT come through here — they
 *    arrive as a 200 with per-op `results`, handled on the success path.
 */
function isPermanentPushError(status?: number): boolean {
  if (status === undefined) return false;
  return (
    status === 400 ||
    status === 403 ||
    status === 404 ||
    status === 409 ||
    status === 422
  );
}

function stableStringify(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) {
    return "[" + value.map((v) => stableStringify(v)).join(",") + "]";
  }
  const obj = value as Record<string, unknown>;
  const keys = Object.keys(obj).sort();
  const parts = keys.map(
    (k) => JSON.stringify(k) + ":" + stableStringify(obj[k]),
  );
  return "{" + parts.join(",") + "}";
}

// ---------------------------------------------------------------------------
// Convenience factory
// ---------------------------------------------------------------------------

/**
 * One-shot guard: detect the most common production misconfig — a
 * deployed app running on HTTPS with `baseUrl` still pointing at a
 * `localhost` API. Symptom in the wild: blank screen, "Failed to
 * fetch" in DevTools, browser blocking mixed-content WS upgrades.
 *
 * The check fires once per page load and is browser-only (server-
 * side renders see localhost as a legitimate target). It's a loud
 * `console.error` block — surfaces in DevTools, Vercel deploy logs,
 * and Sentry-style trackers — but doesn't throw, so existing apps
 * can't lock up on a misconfigured deploy.
 */
let warnedMisconfig = false;
function warnIfMisconfigured(baseUrl: string): void {
  if (warnedMisconfig) return;
  if (typeof window === "undefined") return;
  const pageHttps = window.location?.protocol === "https:";
  const apiLocalhost =
    /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0)(:|\/|$)/.test(baseUrl);
  if (!pageHttps || !apiLocalhost) return;
  warnedMisconfig = true;
  // eslint-disable-next-line no-console
  console.error(
    "[pylon] Misconfigured deployment:\n" +
      "  Page is on " + window.location.origin + " (https)\n" +
      "  Pylon baseUrl is " + baseUrl + " (localhost)\n" +
      "\n" +
      "Likely cause: NEXT_PUBLIC_PYLON_URL (or your framework's equivalent)\n" +
      "is unset in this deployment. The client falls back to localhost,\n" +
      "and every API request fails with 'Failed to fetch'.\n" +
      "\n" +
      "Fix: set NEXT_PUBLIC_PYLON_URL=https://<your-pylon-host> in the\n" +
      "deployment env, then redeploy. If your dashboard proxies /api/*\n" +
      "to the backend same-origin, set it to '' (empty string) instead.",
  );
}

/**
 * Create a sync engine connected to the pylon backend.
 *
 * Default `baseUrl` resolution order:
 *  1. Explicit `baseUrl` argument — wins always.
 *  2. `window.location.origin` when running in a browser — same-origin
 *     deployments (Next.js + Vercel rewrites, embedded SPA, etc.) want
 *     this and forgetting to pass it should NOT silently leak
 *     `localhost:4321` requests in production.
 *  3. `http://localhost:4321` — the `pylon dev` default for SSR /
 *     non-browser callers (Node scripts, tests).
 */
export function createSyncEngine(
  baseUrl?: string,
  options?: Partial<SyncEngineConfig>,
): SyncEngine {
  // `??` would treat an empty string as "set" — but `init({ baseUrl: "" })`
  // is the intuitive "use page origin" incantation and must fall back. Use
  // `||` so undefined/null/"" all route to the auto-detected origin.
  const resolved =
    (baseUrl && baseUrl.length > 0 ? baseUrl : undefined) ??
    (typeof window !== "undefined" && window.location?.origin
      ? window.location.origin
      : "http://localhost:4321");
  return new SyncEngine({
    ...(options ?? {}),
    baseUrl: resolved,
  });
}
