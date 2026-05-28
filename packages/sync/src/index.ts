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
import { MultiTabBroker } from "./multi-tab";
import { OpQueue } from "./op-queue";
import { ServerSubscriptions } from "./server-subscriptions";
import { SessionResolver } from "./session-resolver";
import { generateClientId, generateId } from "./ids";
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
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private _connectionStatus: SyncConnectionStatus = "offline";
  /** Monotonic attempt counter for exponential backoff. Reset to 0 on a
   *  successful connection so the next reconnect starts fresh rather than
   *  inheriting the previous storm's cooldown. */
  private reconnectAttempts = 0;
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
   * Multi-tab coordinator. One tab in this origin owns the WS + the
   * network ops; the rest are passive mirrors that receive applied
   * changes via BroadcastChannel and forward their mutations to the
   * leader for pushing. Initialized in `start()` so apps that never
   * call `start()` (SSR snapshots, hydrate-only) don't pay for it.
   */
  private multiTab: MultiTabBroker | null = null;
  /** True after the broker has decided this tab is the leader. Used
   *  to gate WS / pull / push / poll. Followers stay passive. */
  private isMultiTabLeader = true;

  /**
   * Timer for the "stable connection" check. On `onopen` we start a 5s
   * timer; if the socket stays up that long we reset reconnectAttempts.
   * If it closes first, the timer gets cleared and the backoff grows so
   * the client can't hammer the server on auth failures.
   */
  private wsStableTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;

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

  /** Per-row local refcount for CRDT subscriptions — N `useLoroDoc`
   *  components on the same `(entity, rowId)` in THIS tab share a
   *  single server subscription. Distinct from reactiveSubOwners
   *  because CRDT keys are per-row (many consumers per tab) while
   *  reactive sub_ids are per-consumer-instance (typically one).
   *  StrictMode double-subscribe still bumps the count by one each
   *  call; the matching double-unsubscribe early-returns when the
   *  count is already zero, so the math balances. */
  private crdtSubscribers: Map<string, number> = new Map();

  /** Per-row set of FOLLOWER tabIds that have forwarded a
   *  `sub-register` for this key. Leader-only. Used to:
   *    (a) skip the WS unsubscribe until both `crdtSubscribers`
   *        and this set are empty, and
   *    (b) skip the cross-tab binary broadcast when no follower
   *        cares about CRDT (saves bandwidth when the leader is
   *        the only CRDT consumer). */
  private crdtForwarders: Map<string, Set<string>> = new Map();

  /** Sentinel for "this leader tab subscribed locally" — used as a
   *  refcount-bearer key in subscription owner sets so the leader's
   *  own subs don't get conflated with forwarded follower subs. */
  private static readonly OWN_TAB = "__self__";

  /** Per-sub_id ownership set for reactive subscriptions on the
   *  leader: which tabs (self + forwarders) want this sub alive.
   *  A SET, not a count, so a follower crash + late `sub-unregister`
   *  storm can't underflow the count, and a remount/StrictMode
   *  double-subscribe from one tab still counts as one owner.
   *
   *  Without per-tab tracking, two `subscribeReactive` calls from
   *  the same tab (e.g., React StrictMode's intentional double-
   *  invocation) would push the count to 2; the matching double-
   *  unsubscribe only decrements once (the second early-returns on
   *  StrictMode-safe semantics), leaking one ref per cycle. */
  private reactiveSubOwners: Map<string, Set<string>> = new Map();

  /** Inbound message routing for reactive subscriptions — server pushes
   *  `reactive-result` / `reactive-error` envelopes keyed by sub_id,
   *  and the React hook's handler lives here. ServerSubscriptions
   *  doesn't route messages; this map does. */
  private reactiveHandlers: Map<string, (msg: ReactiveMessage) => void> =
    new Map();

  /** Specs (fn_name + args) for every reactive subscription this tab
   *  has registered, regardless of leader/follower role. On promotion
   *  the new leader uses these to register with serverSubs; on a
   *  leader change while we stay a follower, we use them to
   *  re-forward `sub-register` to the new leader. */
  private wantedReactiveSpecs: Map<
    string,
    { fn_name: string; args: unknown }
  > = new Map();

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

    // Load persisted data if available.
    const shouldPersist = this.config.persist !== false && typeof indexedDB !== "undefined";
    if (shouldPersist) {
      try {
        const { IndexedDBPersistence, persistChange } = await import("./persistence");
        this.persistence = new IndexedDBPersistence(this.config.appName);
        await this.persistence.open();

        // Load cached data into the store.
        const cached = await this.persistence.loadAllEntities();
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
        if (hydrated) this.store.notify();
        else this.store.notify(); // notify even on empty cache so useQuery
        // sees `isHydrated()` flip and can drop its initial loading state.

        // Load cursor.
        const savedCursor = await this.persistence.loadCursor();
        if (savedCursor) {
          this.cursor = savedCursor;
        }

        // Auto-save changes to IndexedDB. Returns a Promise so the async
        // apply path (applyChangesAsync) can await the write before the
        // cursor advances — the fix for "cursor ahead of replica" on crash.
        const persistence = this.persistence;
        this.store._persistFn = async (change: ChangeEvent) => {
          const { persistChange } = await import("./persistence");
          if (persistence) await persistChange(persistence, change);
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
          // Fire-and-forget — the actual mutation HTTP calls happen
          // async, and we don't want to block engine startup on them.
          // pull()/reconcile() below run in parallel; push()'s
          // mutations carry op_ids so racing the broadcasts won't
          // double-apply.
          void this.push();
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
    await this.initMultiTab();

    if (!this.isMultiTabLeader) {
      // Follower path: rely on the leader's broadcasts for session +
      // applied changes. Nothing else to do here — the broker is
      // wired to forward inbound messages into the engine.
      return;
    }

    // Seed the server-resolved session before the first pull so
    // `useSession` subscribers see the right tenant from frame one,
    // and the resolver's lastSeenTenant is populated before any
    // subsequent flip can race with it.
    await this.refreshResolvedSession();

    // Pull from server, then connect real-time transport.
    await this.pull();

    // Save cursor after pull.
    if (this.persistence) {
      await this.persistence.saveCursor(this.cursor);
    }

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

    const transport = this.config.transport ?? "websocket";
    if (transport === "websocket") {
      this.connectWs();
    } else if (transport === "sse") {
      this.connectSse();
    } else if (transport === "poll") {
      this.startPolling();
    }
  }

  /**
   * Multi-tab election. Sets `isMultiTabLeader` and (when leader)
   * wires up a broker that broadcasts every applied change so
   * follower tabs converge without their own WS.
   *
   * On platforms without BroadcastChannel (Node, jsdom, very old
   * Safari) every tab is implicitly its own leader and this method
   * returns immediately.
   */
  private async initMultiTab(): Promise<void> {
    if (this.config.multiTab === false) {
      this.isMultiTabLeader = true;
      return;
    }
    if (!MultiTabBroker.available()) {
      this.isMultiTabLeader = true;
      return;
    }
    const channelName = `pylon:${this.config.appName ?? "default"}:multitab`;
    this.multiTab = new MultiTabBroker();
    let settled = false;
    let promotedDuringStart = false;
    await new Promise<void>((resolve) => {
      const finish = () => {
        if (settled) return;
        settled = true;
        resolve();
      };
      this.multiTab!.start(channelName, {
        onPromote: () => {
          this.isMultiTabLeader = true;
          if (!settled) {
            promotedDuringStart = true;
            finish();
          } else {
            void this.onMultiTabPromoted();
          }
        },
        onDemote: () => {
          this.isMultiTabLeader = false;
          this.onMultiTabDemoted();
        },
        onAppMessage: (payload, from) =>
          this.handleMultiTabMessage(payload, from.tabId),
      });
      // Bound the settle wait — if no other tab claims leader within
      // the election window, this tab takes the role.
      setTimeout(() => {
        if (!settled) {
          // Broker hasn't fired onPromote yet (e.g., it's still
          // waiting for the settle timer). Force it: ask broker for
          // current status; if it says we're leader, treat as promoted.
          if (this.multiTab!.isLeader()) {
            this.isMultiTabLeader = true;
            promotedDuringStart = true;
          }
          finish();
        }
      }, 400);
    });
    void promotedDuringStart;
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
      for (const [sub_id, spec] of this.wantedReactiveSpecs) {
        this.serverSubs.register(sub_id, {
          type: "reactive-subscribe",
          sub_id,
          fn_name: spec.fn_name,
          args: spec.args,
        });
      }
      for (const key of this.crdtSubscribers.keys()) {
        const [entity, rowId] = key.split("\x00");
        if (entity && rowId !== undefined) {
          this.serverSubs.register(key, {
            type: "crdt-subscribe",
            entity,
            rowId,
          });
        }
      }
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
      const transport = this.config.transport ?? "websocket";
      if (transport === "websocket") this.connectWs();
      else if (transport === "sse") this.connectSse();
      else if (transport === "poll") this.startPolling();
    } catch {
      /* best-effort — next reconnect cycle catches up */
    }
  }

  /** Replay every server-subscription this tab is interested in by
   *  re-broadcasting `sub-register` messages. Triggered by a new
   *  leader asking via `request-sub-replay`. The leader's handler
   *  registers them with its serverSubs. */
  private replayForwardedSubs(): void {
    for (const [sub_id, spec] of this.wantedReactiveSpecs) {
      this.broadcastToTabs({
        type: "sub-register",
        kind: "reactive",
        key: sub_id,
        sub_id,
        fn_name: spec.fn_name,
        args: spec.args,
      });
    }
    for (const key of this.crdtSubscribers.keys()) {
      const [entity, rowId] = key.split("\x00");
      if (entity && rowId !== undefined) {
        this.broadcastToTabs({
          type: "sub-register",
          kind: "crdt",
          key,
          entity,
          rowId,
        });
      }
    }
  }

  /** Demotion: another tab took over as leader. Close our WS and
   *  stop driving network ops; the new leader will broadcast applied
   *  changes that our followers-mirror path picks up. */
  private onMultiTabDemoted(): void {
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        /* best-effort */
      }
      this.ws = null;
    }
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
  }

  /** Inbound message from another tab in this origin. The leader's
   *  broadcasts feed follower replicas; followers' requests feed the
   *  leader's mutation queue + subscription registry. */
  private handleMultiTabMessage(payload: unknown, fromTabId: string): void {
    if (!payload || typeof payload !== "object") return;
    const msg = payload as { type?: string } & Record<string, unknown>;
    switch (msg.type) {
      case "applied": {
        // Leader → followers: a batch of change events just landed.
        // Run through our own apply path so the seq filter dedupes
        // anything that also reached us via our own (defunct) WS.
        // `fromBroadcast: true` suppresses re-broadcast in case a
        // promotion lands between this enqueue and the apply.
        const changes = msg.changes as ChangeEvent[] | undefined;
        const targetCursor = msg.targetCursor as SyncCursor | undefined;
        if (Array.isArray(changes) && changes.length > 0) {
          void this.enqueueApply(changes, targetCursor, { fromBroadcast: true });
        } else if (targetCursor && targetCursor.last_seq > this.cursor.last_seq) {
          this.cursor = targetCursor;
        }
        break;
      }
      case "reconciled": {
        // Leader → followers: a reconcile batch landed for this
        // entity. Apply locally with the same tombstone semantics.
        const entity = msg.entity as string | undefined;
        const upserts = msg.upserts as Row[] | undefined;
        const removalIds = msg.removalIds as string[] | undefined;
        const tombstoneSeq = msg.tombstoneSeq as number | undefined;
        if (
          typeof entity === "string" &&
          Array.isArray(upserts) &&
          Array.isArray(removalIds) &&
          typeof tombstoneSeq === "number"
        ) {
          void this.enqueueReconcile(entity, upserts, removalIds, tombstoneSeq, {
            fromBroadcast: true,
          });
        }
        break;
      }
      case "reset": {
        // Leader → followers: replica was reset (identity flip).
        void this.resetReplicaInner();
        break;
      }
      case "session": {
        // Leader → followers: /api/auth/me result. Funnel through
        // the shared session chain so concurrent broadcasts (or a
        // broadcast racing with a local notifySessionChanged) commit
        // in arrival order — without that, the older tenant could
        // win and pin the engine to a stale session.
        const resolved = msg.resolved as ResolvedSession | undefined;
        if (resolved) {
          void this.applySessionTransition(resolved, /* broadcast */ false);
        }
        break;
      }
      case "mutations": {
        // Follower → leader: ops the follower wants pushed. Add to
        // our queue and trigger a push. The op_ids ensure server-side
        // dedupe is intact even if the follower also retries.
        if (!this.isMultiTabLeader) return;
        const ops = msg.ops as PendingMutation[] | undefined;
        if (Array.isArray(ops)) {
          for (const op of ops) {
            this.mutations.add(op.change);
          }
          void this.push();
        }
        break;
      }
      case "mutations-acked": {
        // Leader → followers: these op_ids were pushed successfully.
        // Followers may carry the same ops in their local queue (their
        // optimistic apply seeded them); clearing prevents re-forwarding
        // on the next push trigger.
        const opIds = msg.opIds as string[] | undefined;
        if (Array.isArray(opIds)) {
          for (const id of opIds) this.mutations.markApplied(id);
          this.mutations.clear();
        }
        break;
      }
      case "sub-register": {
        // Follower → leader: register a WS subscription on the
        // follower's behalf. The leader's serverSubs sends the
        // subscribe frame; inbound `reactive-result` / binary frames
        // already broadcast back to all tabs (see WS onmessage).
        if (!this.isMultiTabLeader) return;
        const kind = msg.kind as string;
        const key = msg.key as string;
        if (kind === "crdt") {
          const entity = msg.entity as string;
          const rowId = msg.rowId as string;
          // Track the forwarding follower in a separate set (NOT
          // the local refcount) — repeated sub-register from the
          // same follower stays idempotent, and a follower crash
          // before sub-unregister leaks at most one entry that the
          // next election drops from the roster.
          let fwd = this.crdtForwarders.get(key);
          if (!fwd) {
            fwd = new Set();
            this.crdtForwarders.set(key, fwd);
          }
          fwd.add(fromTabId);
          // Register if nothing else owned this key — local count
          // and forwarder set together gate the WS subscribe.
          const localCount = this.crdtSubscribers.get(key) ?? 0;
          if (localCount === 0 && fwd.size === 1) {
            this.serverSubs.register(key, {
              type: "crdt-subscribe",
              entity,
              rowId,
            });
          }
        } else if (kind === "reactive") {
          const sub_id = msg.sub_id as string;
          const fn_name = msg.fn_name as string;
          const args = msg.args;
          // Per-tab ownership: the forwarding follower adds itself
          // to the owner set. Re-broadcasts from the SAME tab (e.g.,
          // StrictMode remount + sub-register) leave the size at 1.
          let owners = this.reactiveSubOwners.get(sub_id);
          if (!owners) {
            owners = new Set();
            this.reactiveSubOwners.set(sub_id, owners);
          }
          owners.add(fromTabId);
          this.serverSubs.register(sub_id, {
            type: "reactive-subscribe",
            sub_id,
            fn_name,
            args,
          });
        }
        break;
      }
      case "sub-unregister": {
        if (!this.isMultiTabLeader) return;
        const kind = msg.kind as string;
        const key = msg.key as string;
        if (kind === "crdt") {
          const entity = msg.entity as string;
          const rowId = msg.rowId as string;
          const fwd = this.crdtForwarders.get(key);
          if (fwd) {
            fwd.delete(fromTabId);
            if (fwd.size === 0) this.crdtForwarders.delete(key);
          }
          // Unsubscribe only when neither local consumers nor any
          // forwarding follower still wants this row.
          const localCount = this.crdtSubscribers.get(key) ?? 0;
          const remainingFwd = this.crdtForwarders.get(key)?.size ?? 0;
          if (localCount === 0 && remainingFwd === 0) {
            if (this.serverSubs.has(key)) {
              this.serverSubs.unregister(key, {
                type: "crdt-unsubscribe",
                entity,
                rowId,
              });
            }
          }
        } else if (kind === "reactive") {
          const sub_id = msg.sub_id as string;
          const owners = this.reactiveSubOwners.get(sub_id);
          if (owners) {
            owners.delete(fromTabId);
            if (owners.size === 0) {
              this.reactiveSubOwners.delete(sub_id);
              if (this.serverSubs.has(sub_id)) {
                this.serverSubs.unregister(sub_id, {
                  type: "reactive-unsubscribe",
                  sub_id,
                });
              }
            }
          }
        }
        break;
      }
      case "reactive-msg": {
        // Leader → followers: a reactive-result/error envelope landed
        // on the leader's WS. Route to our local handler if we have
        // one registered (we will, if this tab is the follower that
        // requested the subscription).
        const sub_id = msg.sub_id as string;
        const payload = msg.payload as ReactiveMessage;
        const handler = this.reactiveHandlers.get(sub_id);
        if (handler) handler(payload);
        break;
      }
      case "binary": {
        // Leader → followers: a binary frame landed on the leader's
        // WS. Dispatch to local binaryHandlers (the CRDT consumer
        // registers one of these via `onBinaryFrame`).
        const bytes = msg.bytes as Uint8Array | undefined;
        if (bytes instanceof Uint8Array) {
          for (const h of this.binaryHandlers) {
            try {
              h(bytes);
            } catch (err) {
              console.warn("[sync] binary handler threw:", err);
            }
          }
        }
        break;
      }
      case "request-sub-replay": {
        // New leader is asking every follower to re-forward its subs.
        // Only respond if we're a follower — if we're the leader
        // ourselves, we already have them in our serverSubs.
        if (!this.isMultiTabLeader) this.replayForwardedSubs();
        break;
      }
    }
  }

  /** Broadcast a payload to other tabs in this origin. No-op when
   *  the broker isn't running. */
  private broadcastToTabs(payload: unknown): void {
    this.multiTab?.broadcastApp(payload);
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

  private pollTimer: ReturnType<typeof setInterval> | null = null;

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
    opts: { fromBroadcast?: boolean } = {},
  ): Promise<void> {
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
        await this.store.applyChangesAsync(filtered);
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
        this.cursor = candidate;
        if (this.persistence) {
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

  private startPolling(): void {
    const interval = this.config.pollInterval ?? 1000;
    this.pollTimer = setInterval(() => {
      this.push().then(() => this.pull());
    }, interval);
  }

  /** Stop the sync engine. */
  stop(): void {
    this.running = false;
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    if (this.visibilityHandler && typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", this.visibilityHandler);
      this.visibilityHandler = null;
    }
    if (this.multiTab) {
      this.multiTab.stop();
      this.multiTab = null;
    }
    this.setConnectionStatus("offline");
  }

  /** Connect to the WebSocket server for real-time updates. */
  private connectWs(): void {
    if (!this.running) return;
    // Followers never open a WS — the leader's fanout is mirrored
    // over the BroadcastChannel.
    if (!this.isMultiTabLeader) return;

    const wsUrl = this.config.wsUrl ?? this.deriveWsUrl();
    // Browser WebSocket has no header API — the server accepts the token
    // as a `bearer.<percent-encoded-token>` subprotocol (RFC 6455 §1.9).
    // Native clients can still set Authorization: Bearer via headers.
    const token =
      this.config.token ??
      this.storage.get(this.tokenStorageKey()) ??
      undefined;
    try {
      if (token) {
        const proto = `bearer.${encodeURIComponent(token)}`;
        this.ws = new WebSocket(wsUrl, proto);
      } else {
        this.ws = new WebSocket(wsUrl);
      }
    } catch {
      this.scheduleReconnect();
      return;
    }

    // Backoff reset is delayed — a socket that opens then closes inside
    // a few seconds (auth failure, server 1008) would otherwise let the
    // reconnect loop fire at ~2/sec forever. Only call the connection
    // "stable" after it's stayed up long enough to have been doing work.
    this.ws.onopen = () => {
      // We only flip to "connected" once the socket actually opens.
      // The 5s stable-window timer below decides when to RESET the
      // backoff; status flips immediately because UI consumers want
      // to clear the "reconnecting" indicator the moment data starts
      // flowing again.
      this.setConnectionStatus("connected");
      if (this.wsStableTimer) clearTimeout(this.wsStableTimer);
      this.wsStableTimer = setTimeout(() => {
        this.reconnectAttempts = 0;
        this.wsStableTimer = null;
      }, 5_000);
      // Client-side keepalive ping. Default 25s — pure liveness, since
      // the dedicated :port+1 server uses a dual-thread design that
      // wakes the writer instantly on every broadcast (no mutex
      // contention with the reader, no ping-bounded latency).
      //
      // The HTTP-multiplexed `/api/sync/ws` fallback path is still
      // single-threaded (tiny_http's CustomStream hides the TcpStream
      // so we can't set a kernel-level read timeout). On that path,
      // broadcast latency IS bounded by this interval — apps that
      // can't route to the :port+1 listener can pass
      // `init({ pingIntervalMs: 200 })` to trade traffic for latency.
      const pingIntervalMs = this.config.pingIntervalMs ?? 25_000;
      if (this.pingTimer) clearInterval(this.pingTimer);
      this.pingTimer = setInterval(() => {
        if (this.ws?.readyState !== WebSocket.OPEN) return;
        try {
          this.ws.send('{"type":"ping"}');
        } catch {
          // ignore — onclose will trigger reconnect
        }
      }, pingIntervalMs);
      // Re-send every active server-subscription (CRDT rows, reactive
      // queries, future kinds) across the new socket. The server
      // purges per-client subscription state on disconnect, so without
      // this resync the subscriber's first event would never arrive.
      this.serverSubs.replay();
      // Pull-on-open catches every event broadcast in the gap between
      // the prior `pull()` returning and this socket actually opening.
      // The WS has no replay-on-connect (it's just a fanout), so events
      // emitted to other live clients during that window would otherwise
      // be lost forever to this tab. Reconcile fires after the pull
      // since pull is the cheap incremental path; reconcile is the
      // server-truth backstop for anything pull couldn't replay.
      void this.pull().then(() => this.reconcile());
    };

    // Bind binaryType BEFORE installing the handler so the first
    // server-pushed binary frame (CRDT snapshot or update) decodes
    // correctly. Default in browsers is "blob"; we want raw bytes
    // synchronously available so the binary-handler closure doesn't
    // need to await a Blob.arrayBuffer() round-trip.
    this.ws.binaryType = "arraybuffer";

    this.ws.onmessage = (event) => {
      // Binary frame: route to whatever consumer registered via
      // onBinaryFrame(). Pylon's CRDT broadcast (server-side
      // notify_crdt) ships every CRDT-mode write as a binary
      // [type|entity|row_id|payload] frame; @pylonsync/loro is the
      // intended decoder. SyncEngine itself stays binary-agnostic so
      // the next binary use case (file streaming, video chunks…)
      // can register without churning this layer.
      if (event.data instanceof ArrayBuffer) {
        const bytes = new Uint8Array(event.data);
        for (const handler of this.binaryHandlers) {
          try {
            handler(bytes);
          } catch (err) {
            console.warn("[sync] binary handler threw:", err);
          }
        }
        // Forward to follower tabs ONLY when at least one follower
        // is currently forwarded for a CRDT row. The engine is
        // binary-agnostic — it can't peek inside the frame to route
        // per-row — so this is a tab-level gate: no forwarders = no
        // broadcast. Saves bandwidth when the leader is the only
        // CRDT consumer (the common single-tab case).
        if (this.crdtForwarders.size > 0) {
          this.broadcastToTabs({ type: "binary", bytes });
        }
        return;
      }

      try {
        const msg = JSON.parse(event.data as string);

        // Sync change event. Persist BEFORE advancing the cursor so a
        // crash can't leave `last_seq` ahead of the replica on disk.
        // The shared apply queue serializes WS messages with each other
        // AND with concurrent pull() calls, so seq order is preserved
        // and the cursor only advances monotonically.
        if (msg.seq && msg.entity && msg.kind) {
          const change = msg as ChangeEvent;
          void this.enqueueApply([change]);
          return;
        }

        // Presence event.
        if (msg.type === "presence") {
          this.store.notify();
          return;
        }

        // Server-driven revocation: a subscriber whose read policy
        // was revoked mid-session for a specific row. Drop the row
        // from the local replica at the current cursor seq so the
        // tombstone supersedes any racing late-arriving WS update
        // for the same row, and notify any LoroDoc subscriber
        // (registered via `addRowEvictionListener`) so collaborative
        // doc handles unmount cleanly.
        //
        // Distinct from a regular Delete change event because this
        // envelope has no global seq — the row's underlying data
        // hasn't been deleted, only the recipient's visibility of
        // it. Other subscribers (with matching policy) keep their
        // row intact.
        if (
          msg.type === "row-revoked" &&
          typeof msg.entity === "string" &&
          typeof msg.row_id === "string"
        ) {
          // Server includes its current high-water seq when known —
          // use it as the tombstone seq so an in-flight stale frame
          // with `seq <= server_seq` is filtered locally. A
          // legitimate re-grant at a higher seq still lands.
          const revokeSeq =
            typeof msg.seq === "number" && msg.seq > 0
              ? msg.seq
              : this.cursor.last_seq;
          this.handleRowRevocation(msg.entity, msg.row_id, revokeSeq);
          return;
        }

        // Session mutated server-side. Fires for select-org / clear-org
        // / session revoke — every tab connected as this user gets the
        // envelope (cross-machine too via the cluster bus). Trigger
        // a fresh /api/auth/me read which updates the cached session
        // AND, on tenant flip, resets the replica so stale rows from
        // the previous tenant disappear. App code calling
        // /api/auth/select-org via raw fetch no longer needs the
        // manual `notifySessionChanged()` step.
        if (msg.type === "session-changed") {
          void this.refreshResolvedSession();
          return;
        }

        // Reactive query push: the server-side ReactiveRegistry re-ran
        // a subscribed handler and the result hash changed. Route to
        // the local handler if we own the subscription, AND forward
        // to follower tabs via the multi-tab channel so a follower's
        // `useReactiveQuery` handler fires too (the follower
        // registered through us via `sub-register`).
        if (msg.type === "reactive-result" && typeof msg.sub_id === "string") {
          const handler = this.reactiveHandlers.get(msg.sub_id);
          if (handler) {
            handler({ kind: "result", result: msg.result });
          }
          this.broadcastToTabs({
            type: "reactive-msg",
            sub_id: msg.sub_id,
            payload: { kind: "result", result: msg.result },
          });
          return;
        }
        if (msg.type === "reactive-error" && typeof msg.sub_id === "string") {
          const errPayload = {
            kind: "error" as const,
            code: typeof msg.code === "string" ? msg.code : "REACTIVE_ERROR",
            message: typeof msg.message === "string" ? msg.message : "",
          };
          const handler = this.reactiveHandlers.get(msg.sub_id);
          if (handler) handler(errPayload);
          this.broadcastToTabs({
            type: "reactive-msg",
            sub_id: msg.sub_id,
            payload: errPayload,
          });
          return;
        }
      } catch {
        // Ignore malformed messages.
      }
    };

    this.ws.onclose = () => {
      this.ws = null;
      // Socket closed before the stable-window timer fired — treat this
      // as an unstable connection and DO NOT reset reconnectAttempts.
      // The growing backoff protects the server from a tight loop.
      if (this.wsStableTimer) {
        clearTimeout(this.wsStableTimer);
        this.wsStableTimer = null;
      }
      if (this.pingTimer) {
        clearInterval(this.pingTimer);
        this.pingTimer = null;
      }
      // Surface the disconnect to UI consumers immediately. If
      // `running` flipped to false (engine stopped), `stop()` already
      // set "offline" — don't override that.
      if (this.running) {
        this.setConnectionStatus("reconnecting");
      }
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      // onclose will fire after this.
    };
  }

  private scheduleReconnect(): void {
    if (!this.running) return;
    this.reconnectAttempts += 1;
    const delay = this.computeBackoff();
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      // Pull any missed changes, then reconnect.
      this.pull().then(() => this.connectWs());
    }, delay);
  }

  /**
   * Exponential backoff with full jitter for reconnects.
   *
   * Thundering-herd fix: when the server restarts, every connected client
   * fires `onclose` at nearly the same instant. Without jitter they all
   * reconnect at `baseDelay` and hammer the newly-booted server; after a
   * few cycles the reconnect waves align and the server never recovers.
   *
   * Full-jitter (`delay = random(0, exp)`) spreads clients evenly across
   * the backoff window so the second-wave load is flat, not spiky.
   * Algorithm from AWS Architecture Blog "Exponential Backoff and Jitter"
   * — the "Full Jitter" variant, which has the lowest collision rate.
   *
   * The `reconnectDelay` config value seeds the exponential base. Max
   * delay caps at 30s so users don't wait minutes on a long outage.
   */
  private computeBackoff(): number {
    const base = this.config.reconnectDelay ?? 1000;
    const maxDelay = 30_000;
    // exp = base * 2^(attempts-1), clamped to maxDelay
    const attempt = Math.max(1, this.reconnectAttempts);
    const exp = Math.min(maxDelay, base * Math.pow(2, attempt - 1));
    // Full jitter: delay is uniform random in [0, exp].
    return Math.floor(Math.random() * exp);
  }

  /** Connect via Server-Sent Events. */
  private connectSse(): void {
    if (!this.running) return;

    const base = this.config.baseUrl;
    const url = new URL(base);
    const port = parseInt(url.port || "4321", 10);
    const sseUrl = `http://${url.hostname}:${port + 2}/events`;

    try {
      const es = new EventSource(sseUrl);
      es.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.seq && msg.entity && msg.kind) {
            const change = msg as ChangeEvent;
            void this.enqueueApply([change]);
          }
        } catch {
          // Ignore malformed events.
        }
      };
      es.onerror = () => {
        es.close();
        // Same jittered backoff as the WS path so SSE clients don't form
        // a second reconnect wave on server restart.
        this.reconnectAttempts += 1;
        setTimeout(() => {
          if (this.running) {
            this.pull().then(() => this.connectSse());
          }
        }, this.computeBackoff());
      };
    } catch {
      // EventSource not available — fall back to polling.
      this.startPolling();
    }
  }

  private deriveWsUrl(): string {
    const base = this.config.baseUrl;
    const url = new URL(base);
    const scheme = url.protocol === "https:" ? "wss" : "ws";

    // Always multiplex WS on the same origin via `/api/sync/ws`. The
    // Pylon runtime accepts the Upgrade on its main HTTP port (4321),
    // so any reverse proxy that already forwards `/api/*` carries the
    // WebSocket through too (Vite's `ws: true` proxy, Next.js rewrites,
    // CDNs with WS support).
    //
    // The legacy port+1 fallback (`:4322` for a `:4321` API) is still
    // available on the runtime, but we don't derive it client-side
    // anymore: any setup where the page origin (e.g. Vite on :3000)
    // wasn't equal to the API origin would compute ws://localhost:3001
    // — which doesn't exist and bypasses the dev-server proxy. The
    // `/api/sync/ws` path goes through whatever proxies `/api/*`,
    // which is the same code path prod already relies on.
    return `${scheme}://${url.host}/api/sync/ws`;
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
  async resetReplica(): Promise<void> {
    // Public callers go through the queue so a reset can't race with
    // an in-flight pull / push / reconcile. Internal callers that
    // already hold the queue slot use `resetReplicaInner` directly.
    return this.opQueue.enqueue("reset", () => this.resetReplicaInner());
  }

  private async resetReplicaInner(): Promise<void> {
    this.cursor = { last_seq: 0 };
    this.store.clearAll();
    if (this.persistence) {
      try {
        await this.persistence.clear();
        await this.persistence.saveCursor(this.cursor);
      } catch {
        /* best-effort */
      }
    }
    // Leader broadcasts the reset so follower replicas wipe their
    // own copies in lockstep — otherwise a follower keeps stale
    // rows under the old identity until its own pull catches up.
    if (this.isMultiTabLeader) {
      this.broadcastToTabs({ type: "reset" });
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
    return this.opQueue.enqueue("pull", () => this.pullInner());
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
      // queue's reset path to avoid self-deadlock.
      await this.resetReplicaInner();
      // Token flipped → the cached tenant is for the previous user. Pull
      // the fresh session in parallel with the cursor catch-up below.
      void this.refreshResolvedSession();
    }

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
        const params = new URLSearchParams();
        params.set("since", String(this.cursor.last_seq));
        if (snapshotAfter) {
          params.set("snapshot_after", snapshotAfter);
        }
        const resp = await this.request<
          PullResponse & { snapshot_after?: string | null }
        >("GET", `/api/sync/pull?${params.toString()}`);
        this.consecutive_410s = 0;
        await this.enqueueApply(resp.changes, resp.cursor);
        // `snapshot_after` is only set when the server is mid-snapshot.
        // Continue paginating in the same loop iteration so we don't
        // leave a fresh client with a partial replica.
        snapshotAfter = resp.snapshot_after ?? undefined;
        // The change-log tail also paginates via `has_more` — handle
        // that one recursively after the snapshot loop completes so
        // backpressure on the change-log path uses the existing
        // tail-pull semantics.
        if (!snapshotAfter && resp.has_more) {
          await this.pull();
          break;
        }
      }
    } catch (err) {
      // Swallow network + transient errors so the poll/reconnect loop
      // keeps trying — but on 429 bump the backoff counter so the next
      // reconnect waits noticeably longer. Without this, a rate-limited
      // pull triggers onclose → scheduleReconnect → pull → 429 in a
      // tight loop that the server reads as abuse.
      const status = (err as { status?: number })?.status;
      if (status === 429) {
        this.reconnectAttempts += 3;
      }
      // 410 RESYNC_REQUIRED: cursor is from a previous server lifetime, or
      // it fell off the retention window. Drop local state + cursor and
      // re-pull from seq=0. The server replays all current entity rows as
      // seed events on startup so the fresh pull reconstructs state.
      //
      // Circuit breaker: if the immediate re-pull ALSO 410s, accept it.
      // Don't recurse — that's the infinite loop we used to ship before
      // the cursor=0 server fix landed (or against an old server binary
      // that hasn't been rebuilt yet). Track 410 retries against an
      // exponential backoff so a misconfigured server can't melt our CPU.
      if (status === 410) {
        const attempt = this.consecutive_410s;
        this.consecutive_410s += 1;
        if (attempt === 0) {
          // Bypass the queue here — we ARE the pull op holding the
          // queue slot. Calling the public pull() would re-enqueue and
          // share our own promise back to us (deadlock).
          await this.resetReplicaInner();
          await this.pullInner();
        } else {
          // Already retried once and still 410. Stop. Schedule a
          // back-off retry tied to the WS reconnect path so we don't
          // spam the server. Resets when any pull succeeds.
          const delayMs = Math.min(30_000, 1000 * 2 ** Math.min(attempt, 5));
          console.warn(
            `[pylon] persistent 410 RESYNC_REQUIRED (attempt ${attempt + 1}); backing off ${delayMs}ms`,
          );
          setTimeout(() => {
            // Trigger one more attempt; either it succeeds (which resets
            // the counter) or it 410s again (which extends the backoff).
            void this.pull();
          }, delayMs);
        }
      }
    }
  }

  /** Consecutive 410 RESYNC_REQUIRED responses since the last successful
   *  pull. Used by the circuit breaker in pull() to bound the retry
   *  storm against a misconfigured server. Resets to 0 on any pull
   *  that doesn't throw a 410. */
  private consecutive_410s = 0;

  /** Timestamp of the last `reconcile()` invocation. Used to debounce —
   *  reconcile runs on connect, WS reconnect, AND visibility-change, so
   *  a quick tab-flick after a normal reconnect shouldn't refetch every
   *  entity twice within seconds. Configurable via `reconcileMinIntervalMs`. */
  private lastReconcileAt = 0;

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
   * no arg, every entity with local rows is checked.
   */
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
    const names = entities ?? this.store.entityNames();
    if (names.length === 0) return;
    // Tombstone seq for any local row the server doesn't return. Using
    // the current cursor means future inserts (which have higher seqs)
    // bypass the tombstone — re-creation server-side still propagates.
    const tombstoneSeq = this.cursor.last_seq;
    for (const entity of names) {
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
      try {
        serverRows = await this.fetchEntityRows(entity);
      } catch (err) {
        // Network errors are expected (offline, transient 5xx). Skip
        // this entity; the next reconcile trigger will retry.
        const status = (err as { status?: number })?.status;
        if (status === 403 || status === 404) {
          // Entity is no longer readable (policy revoked) or removed
          // from the manifest. Drop every local row for it — keeping
          // them around just leaks invisible state.
          await this.dropEntity(entity, tombstoneSeq);
        }
        continue;
      }
      if (this.cursor.last_seq !== cursorBeforeFetch) {
        // Cursor moved during fetch — at least one WS event for this
        // (or another) entity landed and might have a fresher value
        // for a row our snapshot just captured. Bail out for this
        // entity; reconcile() is triggered again on visibility-change
        // and reconnect, and the WS event already carried the latest
        // state for the affected row.
        continue;
      }
      if (this.session.signature() !== sessionBeforeFetch) {
        // Session changed (token flipped, tenant switched, user
        // signed out → in, etc.). The rows we fetched reflect the
        // OLD session's policy view; applying them now would
        // tombstone rows visible under the NEW session. Bail and let
        // the session-changed envelope drive the next reconcile.
        continue;
      }
      await this.applyEntityReconcile(entity, serverRows, tombstoneSeq);
    }
  }

  /** Fetch every row for an entity. Uses cursor pagination so big tables
   *  don't blow past server-side limits; loops until `has_more` is false
   *  or a safety cap is hit. */
  private async fetchEntityRows(entity: string): Promise<Row[]> {
    const out: Row[] = [];
    let cursor: string | null = null;
    // 200 pages × 100 per page = 20k rows. Anything larger should not be
    // mirrored client-side anyway — see useInfiniteQuery for huge tables.
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
      if (!resp.has_more || !resp.next_cursor) break;
      cursor = resp.next_cursor;
    }
    return out;
  }

  private async applyEntityReconcile(
    entity: string,
    serverRows: Row[],
    tombstoneSeq: number,
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
    const removalIds: string[] = [];
    for (const local of this.store.list(entity)) {
      const id = (local as { id?: unknown }).id;
      if (typeof id !== "string") continue;
      if (pendingKeys.has(`${entity}/${id}`)) continue;
      if (!serverIds.has(id)) removalIds.push(id);
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
    let next: ResolvedSession;
    try {
      const res = await this.rawFetch("/api/auth/me");
      if (!res.ok) return;
      const raw = (await res.json()) as {
        user_id?: string | null;
        tenant_id?: string | null;
        is_admin?: boolean;
        roles?: string[];
      };
      next = {
        userId: raw.user_id ?? null,
        tenantId: raw.tenant_id ?? null,
        isAdmin: raw.is_admin ?? false,
        roles: raw.roles ?? [],
      };
    } catch {
      // Swallow — /api/auth/me errors are transient and the next pull
      // will retry. Don't take down the sync loop for this.
      return;
    }
    await this.applySessionTransition(next, /* broadcast */ true);
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
          // the new identity.
          await this.resetReplica();
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
            this.mutations.markFailed(m.id, msg);
          }
        }
      } else {
        // Legacy server response (pre-0.3.188): count-based mapping.
        // Buggy on partial failures but the best we can do without
        // the per-op envelope.
        for (let i = 0; i < pending.length; i++) {
          if (i < resp.applied) {
            this.mutations.markApplied(pending[i].id);
          } else if (resp.errors[i - resp.applied]) {
            this.mutations.markFailed(
              pending[i].id,
              resp.errors[i - resp.applied],
            );
          }
        }
      }

      // Broadcast acks BEFORE clearing locally so followers can match
      // op_ids against their pending queues.
      const ackedOpIds = pending
        .map((m) => m.change.op_id)
        .filter((id): id is string => typeof id === "string");
      if (ackedOpIds.length > 0) {
        this.broadcastToTabs({ type: "mutations-acked", opIds: ackedOpIds });
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
    } catch {
      // Will retry on next tick. op_id makes retries idempotent on the server.
    }
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
    this.store.optimisticUpdate(entity, id, data);
    this.mutations.add({
      entity,
      row_id: id,
      kind: "update",
      data: data as Row,
    });
    await this.push();
  }

  /** Delete a row with optimistic local update. */
  async delete(entity: string, id: string): Promise<void> {
    this.store.optimisticDelete(entity, id);
    this.mutations.add({
      entity,
      row_id: id,
      kind: "delete",
    });
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

  /** Whether the WebSocket is currently connected. */
  get connected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
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
    const key = `${entity}\x00${rowId}`;
    const prev = this.crdtSubscribers.get(key) ?? 0;
    this.crdtSubscribers.set(key, prev + 1);
    if (prev === 0) {
      if (!this.isMultiTabLeader) {
        // Followers don't have a WS — ask the leader to subscribe on
        // our behalf. The leader echoes binary frames back over the
        // broadcast channel so our local binaryHandlers fire normally.
        this.broadcastToTabs({
          type: "sub-register",
          kind: "crdt",
          key,
          entity,
          rowId,
        });
      } else {
        // Leader path: only send the WS subscribe if no follower had
        // already forwarded one for this key (in which case the WS
        // sub is already alive).
        const hasFwd = (this.crdtForwarders.get(key)?.size ?? 0) > 0;
        if (!hasFwd) {
          this.serverSubs.register(key, {
            type: "crdt-subscribe",
            entity,
            rowId,
          });
        }
      }
    }
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
    const key = `${entity}\x00${rowId}`;
    const prev = this.crdtSubscribers.get(key) ?? 0;
    if (prev <= 0) return;
    if (prev === 1) {
      this.crdtSubscribers.delete(key);
      if (!this.isMultiTabLeader) {
        this.broadcastToTabs({
          type: "sub-unregister",
          kind: "crdt",
          key,
          entity,
          rowId,
        });
      } else {
        // Leader: only tear down the WS sub if no follower is still
        // forwarded for this key.
        const remainingFwd = this.crdtForwarders.get(key)?.size ?? 0;
        if (remainingFwd === 0 && this.serverSubs.has(key)) {
          this.serverSubs.unregister(key, {
            type: "crdt-unsubscribe",
            entity,
            rowId,
          });
        }
      }
    } else {
      this.crdtSubscribers.set(key, prev - 1);
    }
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
    // True idempotency under remount / StrictMode double-invocation:
    // sets carry their own dedupe, and the matching double-unmount
    // only decrements once (the second unmount early-returns because
    // we delete the handler on the first), so the math balances
    // without a second-subscribe guard.
    this.reactiveHandlers.set(sub_id, handler);
    this.wantedReactiveSpecs.set(sub_id, { fn_name, args });
    if (!this.isMultiTabLeader) {
      // Follower path: the WS lives on the leader. Forward the spec
      // there; the leader registers with its own serverSubs and
      // echoes inbound `reactive-result` / `reactive-error` envelopes
      // back to us via the channel so our local handler fires.
      this.broadcastToTabs({
        type: "sub-register",
        kind: "reactive",
        key: sub_id,
        sub_id,
        fn_name,
        args,
      });
      return;
    }
    // Leader path: register own ownership (idempotent via the
    // Set) and forward to the WS. ServerSubscriptions re-sends on
    // payload change, so an args update lands.
    let owners = this.reactiveSubOwners.get(sub_id);
    if (!owners) {
      owners = new Set();
      this.reactiveSubOwners.set(sub_id, owners);
    }
    owners.add(SyncEngine.OWN_TAB);
    this.serverSubs.register(sub_id, {
      type: "reactive-subscribe",
      sub_id,
      fn_name,
      args,
    });
  }

  /** Tear down a reactive subscription. Sends the unsubscribe to the
   *  server and clears local state. No-op for unknown sub_ids — React
   *  StrictMode double-unmount won't error. */
  unsubscribeReactive(sub_id: string): void {
    if (!this.reactiveHandlers.has(sub_id)) return;
    this.reactiveHandlers.delete(sub_id);
    this.wantedReactiveSpecs.delete(sub_id);
    if (!this.isMultiTabLeader) {
      this.broadcastToTabs({
        type: "sub-unregister",
        kind: "reactive",
        key: sub_id,
        sub_id,
      });
      return;
    }
    // Leader path: drop self from the owner set; unsubscribe only
    // when no owner (self or forwarder) remains.
    const owners = this.reactiveSubOwners.get(sub_id);
    if (owners) {
      owners.delete(SyncEngine.OWN_TAB);
      if (owners.size === 0) {
        this.reactiveSubOwners.delete(sub_id);
        if (this.serverSubs.has(sub_id)) {
          this.serverSubs.unregister(sub_id, {
            type: "reactive-unsubscribe",
            sub_id,
          });
        }
      }
    }
  }

  private sendWs(msg: unknown): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
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
