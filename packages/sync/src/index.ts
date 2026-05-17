// ---------------------------------------------------------------------------
// pylon sync client
//
// JSON live-query sync with optimistic mutations and an offline write queue.
// CRDT-mode rows travel through the same WebSocket as binary Loro frames,
// but this package intentionally keeps binary payloads opaque and routes
// them to consumers such as @pylonsync/loro. See docs/SYNC.md for the full
// projection + convergence model.
// ---------------------------------------------------------------------------

export { IndexedDBPersistence, persistChange } from "./persistence";
export {
  defaultStorage,
  createWriteThroughStorage,
  type Storage,
} from "./storage";

import { defaultStorage } from "./storage";

export interface ChangeEvent {
  seq: number;
  entity: string;
  row_id: string;
  kind: "insert" | "update" | "delete";
  data?: Record<string, unknown>;
  timestamp: string;
}

export interface SyncCursor {
  last_seq: number;
}

export interface PullResponse {
  changes: ChangeEvent[];
  cursor: SyncCursor;
  has_more: boolean;
}

/**
 * Server-resolved auth/session state. Shape mirrors what `/api/auth/me`
 * returns (which is `AuthContext` from the Rust side, with camelCase
 * normalization on the way out).
 *
 * `userId=null` means anonymous. `tenantId=null` means the user hasn't
 * selected an org yet (or the backend is single-tenant).
 */
export interface ResolvedSession {
  userId: string | null;
  tenantId: string | null;
  isAdmin: boolean;
  roles: string[];
}

export interface PushResponse {
  applied: number;
  errors: string[];
  cursor: SyncCursor;
}

export interface ClientChange {
  entity: string;
  row_id: string;
  kind: "insert" | "update" | "delete";
  data?: Record<string, unknown>;
  /**
   * Client-minted idempotency key. The server tracks recently-seen op_ids
   * and returns a no-op success for replays. Supply this on every retry of
   * the same logical mutation — the `MutationQueue` does so automatically.
   */
  op_id?: string;
}

/**
 * Reactive subscription spec — what the server needs to replay a
 * subscription if the client reconnects. Cached client-side so the
 * `ws.onopen` reconnect sweep can re-register every active sub
 * without the React hooks having to know about reconnect lifecycle.
 */
export interface ReactiveSpec {
  fn_name: string;
  args: unknown;
}

/**
 * Push message routed to a reactive subscription handler. `result`
 * fires on initial run + every time the server's re-run produces a
 * value whose hash differs from the last push. `error` fires when
 * the server can't execute the handler (function not registered,
 * reactive runtime unavailable, runtime error in user code).
 */
export type ReactiveMessage =
  | { kind: "result"; result: unknown }
  | { kind: "error"; code: string; message: string };

// ---------------------------------------------------------------------------
// Local store — in-memory replica of server state
// ---------------------------------------------------------------------------

export type Row = Record<string, unknown>;

export class LocalStore {
  private tables: Map<string, Map<string, Row>> = new Map();
  /**
   * Tombstones: `(entity, row_id) -> deletedAt seq`. A row whose id is in
   * here has been deleted; any insert/update event older than the tombstone
   * is ignored so an out-of-order replay cannot resurrect it.
   *
   * Without tombstones, a delete followed by a reconnect-driven replay of
   * the original insert would re-materialize the row — "last write wins"
   * was decided by arrival order instead of event sequence.
   *
   * The tombstone seq comes from the server's `ChangeEvent.seq`. Client-
   * triggered optimistic deletes use `Number.MAX_SAFE_INTEGER` so they
   * dominate anything a concurrent pull could replay.
   */
  private tombstones: Map<string, Map<string, number>> = new Map();
  private listeners: Set<() => void> = new Set();

  /** Get all rows for an entity. */
  list(entity: string): Row[] {
    const table = this.tables.get(entity);
    if (!table) return [];
    return Array.from(table.values());
  }

  /** Get a row by ID. */
  get(entity: string, id: string): Row | null {
    return this.tables.get(entity)?.get(id) ?? null;
  }

  /** Snapshot of every entity name with at least one local row. Used by
   *  `SyncEngine.reconcile` to know which tables to diff against the
   *  server's current truth. Returning a fresh array lets callers iterate
   *  without holding a reference into the live map. */
  entityNames(): string[] {
    const names: string[] = [];
    for (const [name, table] of this.tables) {
      if (table.size > 0) names.push(name);
    }
    return names;
  }

  /**
   * Remove a row recorded as deleted by the server-truth reconciler.
   * Records a tombstone at `tombstoneSeq` so a stale insert/update
   * replayed afterwards (e.g. from a slow WS frame) doesn't resurrect
   * it. Callers pass the current sync cursor as `tombstoneSeq` — any
   * future change events will have higher seqs and pass the tombstone
   * check; older replays will be filtered.
   *
   * Differs from `optimisticDelete` which uses `MAX_SAFE_INTEGER` (the
   * caller is asserting it knows the future). Reconciliation only knows
   * what the server currently shows; a row re-created server-side later
   * MUST be allowed back in.
   */
  reconcileRemove(entity: string, id: string, tombstoneSeq: number): boolean {
    const table = this.tables.get(entity);
    if (!table || !table.has(id)) return false;
    table.delete(id);
    this.recordTombstone(entity, id, tombstoneSeq);
    return true;
  }

  /** Check if `(entity, id)` has a tombstone. */
  private isTombstoned(entity: string, id: string, at_seq?: number): boolean {
    const tombSeq = this.tombstones.get(entity)?.get(id);
    if (tombSeq === undefined) return false;
    // If the caller didn't tell us when their change happened, treat as
    // "this change is older than the tombstone". Safer default.
    if (at_seq === undefined) return true;
    return at_seq < tombSeq;
  }

  private recordTombstone(entity: string, id: string, seq: number): void {
    if (!this.tombstones.has(entity)) {
      this.tombstones.set(entity, new Map());
    }
    const existing = this.tombstones.get(entity)!.get(id);
    if (existing === undefined || seq > existing) {
      this.tombstones.get(entity)!.set(id, seq);
    }
  }

  /** Apply a change event to the local store. */
  applyChange(change: ChangeEvent): void {
    if (!this.tables.has(change.entity)) {
      this.tables.set(change.entity, new Map());
    }
    const table = this.tables.get(change.entity)!;

    // Drop insert/update events that arrive AFTER a delete for the same row.
    // The tombstone map records the seq of the delete; anything strictly
    // older than that seq is a stale resurrect and must be ignored.
    if (
      (change.kind === "insert" || change.kind === "update") &&
      this.isTombstoned(change.entity, change.row_id, change.seq)
    ) {
      return;
    }

    switch (change.kind) {
      case "insert":
        if (change.data) {
          // Spread data FIRST, then force id = change.row_id. Previously
          // id came first and was overridden by any id field in data,
          // which let a crafted/buggy server event corrupt the replica's
          // primary key on reload.
          table.set(change.row_id, {
            ...change.data,
            id: change.row_id,
          });
        }
        break;
      case "update":
        if (change.data) {
          const existing = table.get(change.row_id) ?? { id: change.row_id };
          table.set(change.row_id, {
            ...existing,
            ...change.data,
            id: change.row_id, // authoritative — ignore any id in data
          });
        }
        break;
      case "delete":
        table.delete(change.row_id);
        this.recordTombstone(change.entity, change.row_id, change.seq);
        break;
    }
  }

  /** Apply multiple changes synchronously. Persistence runs fire-and-forget.
   *  Prefer [`applyChangesAsync`] when you plan to advance a cursor after —
   *  otherwise a crash can save the cursor before rows hit disk, causing
   *  permanent missed changes on restart. */
  applyChanges(changes: ChangeEvent[]): void {
    for (const change of changes) {
      this.applyChange(change);
    }
    this.notify();

    if (this._persistFn) {
      for (const change of changes) {
        // Persist from the post-merge row in memory so updates don't
        // overwrite the on-disk mirror with just the patched columns.
        // `applyChange` already merged update.data into the existing row
        // (see case "update" above); the raw `change.data` only contains
        // the patch and would drop every other column on save.
        const merged = this.hydrateFromMemory(change);
        void this._persistFn(merged);
      }
    }
  }

  /**
   * Apply + persist, awaiting disk writes before returning. Callers that are
   * about to advance a cursor based on `changes` MUST use this path —
   * otherwise cursor durability is broken: a crash between the memory apply
   * and the eventual disk write can persist a cursor that's ahead of the
   * replica, skipping those rows forever on restart.
   */
  async applyChangesAsync(changes: ChangeEvent[]): Promise<void> {
    for (const change of changes) {
      this.applyChange(change);
    }
    this.notify();
    if (this._persistFn) {
      const results = changes.map((c) => this._persistFn!(this.hydrateFromMemory(c)));
      await Promise.all(results.map((r) => (r instanceof Promise ? r : Promise.resolve())));
    }
  }

  /**
   * Reshape a change event so its `data` field matches the row as it now
   * exists in memory after `applyChange` merged the patch. Persistence
   * callers (IndexedDB) save the full row, which only works if they
   * receive the full row. Deletes pass through untouched.
   */
  private hydrateFromMemory(change: ChangeEvent): ChangeEvent {
    if (change.kind === "delete") return change;
    const merged = this.tables.get(change.entity)?.get(change.row_id);
    if (!merged) return change;
    return { ...change, data: merged };
  }

  /** Set a persistence callback for auto-saving changes. The return type is
   *  Promise<void> so callers can await. Void-returning callbacks are still
   *  accepted for backwards compatibility (just not awaitable). */
  _persistFn: ((change: ChangeEvent) => void | Promise<void>) | null = null;

  /** Subscribe to store changes. Returns unsubscribe function. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }

  /** Apply an optimistic insert. Returns a temporary ID. */
  optimisticInsert(entity: string, data: Row): string {
    const tempId = `_pending_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    if (!this.tables.has(entity)) {
      this.tables.set(entity, new Map());
    }
    this.tables.get(entity)!.set(tempId, { id: tempId, ...data });
    this.notify();
    return tempId;
  }

  /**
   * Apply an optimistic insert with a caller-provided id.
   *
   * Used by `useMutation({ optimistic })`: the React hook generates a
   * Pylon-shaped id (40-char hex via `generateId()`), threads it
   * through the mutation args as `_optimisticId`, and the server
   * function honors it on `ctx.db.insert("Entity", { id, ... })`.
   * Because the optimistic ghost and the canonical row share the same
   * `row_id`, the WS broadcast that follows the mutation lands as a
   * field-level merge on top of the optimistic — no delete-then-replace
   * flash, no temp-row swap.
   *
   * Different from `optimisticInsert` (above) which mints a `_pending_`
   * id the server can't possibly know about. Use that for fire-and-
   * forget UI affordances, and this one whenever the canonical insert
   * needs to map back to the same row.
   */
  optimisticInsertWithId(entity: string, id: string, data: Row): void {
    if (!this.tables.has(entity)) {
      this.tables.set(entity, new Map());
    }
    this.tables.get(entity)!.set(id, { ...data, id });
    this.notify();
  }

  /**
   * Roll back an optimistic insert without leaving a tombstone.
   *
   * Counterpart to `optimisticInsertWithId`. When a mutation rejects,
   * we want the ghost row gone but we do NOT want a tombstone — a
   * future legitimate insert with the same id (e.g. user retries the
   * mutation, or a workflow eventually creates the row) must not be
   * blocked. `optimisticDelete` records a MAX_SAFE_INTEGER tombstone
   * which is the wrong semantic here; this is just a plain remove.
   */
  rollbackOptimisticInsert(entity: string, id: string): void {
    const removed = this.tables.get(entity)?.delete(id);
    if (removed) this.notify();
  }

  /** Apply an optimistic update. */
  optimisticUpdate(entity: string, id: string, data: Partial<Row>): void {
    const table = this.tables.get(entity);
    if (!table) return;
    const existing = table.get(id);
    if (existing) {
      table.set(id, { ...existing, ...data });
      this.notify();
    }
  }

  /** Apply an optimistic delete. */
  optimisticDelete(entity: string, id: string): void {
    this.tables.get(entity)?.delete(id);
    // Client-side deletes dominate any concurrent server replay until the
    // server confirms; use MAX_SAFE_INTEGER as the tombstone seq. When the
    // server's real delete event arrives it will refresh the tombstone with
    // the authoritative seq (via `recordTombstone`'s max-of).
    this.recordTombstone(entity, id, Number.MAX_SAFE_INTEGER);
    this.notify();
  }

  /**
   * Drop every table + tombstone in-place, then notify. Used by the sync
   * engine's `resetReplica()` on identity flip (token or tenant changed —
   * the old replica reflects a different visible set). Kept on
   * `LocalStore` so the `tables`/`tombstones` maps stay private.
   */
  clearAll(): void {
    this.tables.clear();
    this.tombstones.clear();
    this.notify();
  }
}

// ---------------------------------------------------------------------------
// Pending mutation queue — offline-safe write queue
// ---------------------------------------------------------------------------

export interface PendingMutation {
  id: string;
  change: ClientChange;
  status: "pending" | "applied" | "failed";
  error?: string;
}

/**
 * Optional persistence backend for the mutation queue. The default
 * IndexedDB persistence layer provides `savePending`/`loadPending`/etc.
 * Callers can supply a custom backend for tests or alternative storage.
 */
export interface MutationQueuePersistence {
  saveAll(mutations: PendingMutation[]): Promise<void>;
  loadAll(): Promise<PendingMutation[]>;
}

/**
 * Offline-safe write queue.
 *
 * Before: the queue was memory-only. A tab crash or refresh silently lost
 * every pending write. Now: if a `persistence` backend is provided the queue
 * writes-through on every mutation, and `hydrate()` restores pending/failed
 * mutations on startup. Applied mutations are pruned during `clear()`.
 *
 * The `id` scheme is stable (timestamp + random suffix) and is also used
 * as the server-side `op_id` for idempotent replay. A retried push carrying
 * the same id will short-circuit on the server instead of re-applying.
 */
export class MutationQueue {
  private queue: PendingMutation[] = [];
  private persistence?: MutationQueuePersistence;

  constructor(persistence?: MutationQueuePersistence) {
    this.persistence = persistence;
  }

  /**
   * Attach a persistence backend after construction. The SyncEngine
   * uses this to swap in IndexedDB-backed persistence once the DB
   * has opened (after the constructor runs). Public so it doesn't
   * need a `// @ts-expect-error` to reach in from the same package.
   */
  attachPersistence(persistence: MutationQueuePersistence): void {
    this.persistence = persistence;
  }

  /** Load persisted queue state. Call once at startup. */
  async hydrate(): Promise<void> {
    if (!this.persistence) return;
    try {
      const loaded = await this.persistence.loadAll();
      // Merge in-memory with on-disk. An `add()` that ran while hydrate
      // was awaiting `loadAll()` will already have flushed a snapshot
      // that didn't include the loaded rows — re-flush after merge so
      // disk matches memory again. Without this, a crash between the
      // interleaved add-flush and the next mutation would leave the
      // on-disk snapshot missing the loaded mutations.
      const existingIds = new Set(this.queue.map((m) => m.id));
      let mergedAny = false;
      for (const m of loaded) {
        if (!existingIds.has(m.id)) {
          this.queue.push(m);
          mergedAny = true;
        }
      }
      if (mergedAny) this.flush();
    } catch (err) {
      // Broken storage shouldn't prevent the app from running — warn and
      // degrade to memory-only mode.
      console.warn("[sync] mutation-queue hydrate failed:", err);
    }
  }

  /** Add a pending mutation. Returns the op_id used for server idempotency. */
  add(change: ClientChange): string {
    const id = `mut_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    // Attach op_id on the outgoing ClientChange itself so the server can dedupe.
    const changeWithOp: ClientChange = { ...change, op_id: id };
    this.queue.push({ id, change: changeWithOp, status: "pending" });
    this.flush();
    return id;
  }

  pending(): PendingMutation[] {
    return this.queue.filter((m) => m.status === "pending");
  }

  markApplied(id: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) m.status = "applied";
    this.flush();
  }

  markFailed(id: string, error: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) {
      m.status = "failed";
      m.error = error;
    }
    this.flush();
  }

  /**
   * Prune applied mutations. Failed mutations are KEPT so the UI can surface
   * them to the user and so retries are possible. Previously this dropped
   * failed mutations too, silently discarding server rejections.
   */
  clear(): void {
    this.queue = this.queue.filter(
      (m) => m.status === "pending" || m.status === "failed",
    );
    this.flush();
  }

  /** Remove a specific mutation by id. Used by the UI after user ack of failures. */
  remove(id: string): void {
    this.queue = this.queue.filter((m) => m.id !== id);
    this.flush();
  }

  /** Fire-and-forget persistence write. Errors are logged but not thrown. */
  private flush(): void {
    if (!this.persistence) return;
    // Snapshot the queue before the async write so we don't race a later mutation.
    const snapshot = this.queue.slice();
    this.persistence.saveAll(snapshot).catch((err) => {
      console.warn("[sync] mutation-queue persist failed:", err);
    });
  }
}

// ---------------------------------------------------------------------------
// Sync engine — coordinates pull, push, local store, mutation queue
// ---------------------------------------------------------------------------

export type TransportType = "websocket" | "sse" | "poll";

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
let idCounter = 0;
export function generateId(): string {
  // BigInt to dodge the 2^53 ceiling — `Date.now() * 1_000_000` busts
  // Number.MAX_SAFE_INTEGER for any timestamp past 1973. Hex output is
  // padded to 32 chars so it lex-sorts at width boundaries (a 39-char
  // id sorts before a 40-char one even when the suffix is larger,
  // which would corrupt cursor pagination).
  const nanos = BigInt(Date.now()) * 1_000_000n;
  const seq = idCounter++ >>> 0;
  return nanos.toString(16).padStart(32, "0") + seq.toString(16).padStart(8, "0");
}

function generateClientId(storage: import("./storage").Storage): string {
  const key = "pylon:client_id";
  const existing = storage.get(key);
  if (existing) return existing;
  const fresh = newUuidLike();
  storage.set(key, fresh);
  return fresh;
}

function newUuidLike(): string {
  try {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
      return crypto.randomUUID();
    }
  } catch {
    /* fall through */
  }
  // Fallback: 20 hex chars from random + time.
  const rand = Math.random().toString(36).slice(2, 10);
  const t = Date.now().toString(36);
  return `cl_${t}_${rand}`;
}

/**
 * Coarse connection state for UI consumers.
 *
 * - `connecting`   — engine is starting up; first WS handshake hasn't
 *                    completed yet. Apps typically render their initial
 *                    skeleton during this state.
 * - `connected`    — WS is open and we've stayed open long enough to
 *                    consider it stable (5s on the wire). Live queries
 *                    are receiving real-time updates.
 * - `reconnecting` — WS dropped (network blip, Fly autostop) and the
 *                    engine is backing off + retrying. Live queries
 *                    keep returning the last-known data; mutations
 *                    queue locally and replay on the next connect.
 * - `offline`      — engine has been stopped via `engine.stop()` or
 *                    was never started. No retries pending.
 *
 * The `useSyncStatus` hook in `@pylonsync/react` subscribes to this
 * via the existing store notify channel so re-renders happen
 * automatically without a separate event bus.
 */
export type SyncConnectionStatus =
  | "connecting"
  | "connected"
  | "reconnecting"
  | "offline";

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
   * Uses `undefined` as the "never observed" sentinel so we can distinguish
   * "first pull ever" from "explicitly anonymous". A first pull doesn't
   * reset (nothing to reset), but every later transition — including
   * null→token → does.
   */
  private lastSeenToken: string | null | undefined = undefined;

  /**
   * Latest server-resolved auth/session state. Refreshed on every pull()
   * by fetching /api/auth/me in parallel. Exposed to consumers via
   * `resolvedSession` so React hooks can subscribe via the store.
   *
   * Subscribers re-render when this updates — we reuse the store's
   * notifier rather than introduce a second pub/sub so every change the
   * app cares about goes through one channel.
   */
  private _resolvedSession: ResolvedSession = {
    userId: null,
    tenantId: null,
    isAdmin: false,
    roles: [],
  };
  private lastSeenTenant: string | null | undefined = undefined;

  /**
   * Timer for the "stable connection" check. On `onopen` we start a 5s
   * timer; if the socket stays up that long we reset reconnectAttempts.
   * If it closes first, the timer gets cleared and the backoff grows so
   * the client can't hammer the server on auth failures.
   */
  private wsStableTimer: ReturnType<typeof setTimeout> | null = null;

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
   * Active CRDT subscriptions, keyed `${entity}\x00${rowId}`. Tracked
   * here so a WS reconnect can re-send the same subscriptions to the
   * fresh socket — the server clears its per-client subscription state
   * on disconnect (in `WsHub::handle_ws_connection`'s Close path), so
   * without re-sending the binary frames would stop arriving on the
   * new connection.
   *
   * Refcount-aware via `crdtSubscribers` so two `useLoroDoc` callers on
   * the same row don't unsubscribe each other when one unmounts.
   */
  private crdtSubscriptions: Set<string> = new Set();
  private crdtSubscribers: Map<string, number> = new Map();

  /**
   * Reactive query subscriptions registered via `subscribeReactive`.
   * Two maps:
   *  - `reactiveSpecs`: sub_id → {fn_name, args} for re-registration
   *    on WS reconnect. Server-side state evaporates on disconnect.
   *  - `reactiveHandlers`: sub_id → handler that receives result + error
   *    pushes. The React hook owns these handlers and unsubscribes on
   *    unmount.
   *
   * Both maps are keyed by the same client-minted `sub_id` so they
   * stay in sync. Cleared together by `unsubscribeReactive`.
   */
  private reactiveSpecs: Map<string, ReactiveSpec> = new Map();
  private reactiveHandlers: Map<string, (msg: ReactiveMessage) => void> =
    new Map();

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
    return this._resolvedSession;
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
        if (hydrated) this.store.notify();

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

        // Hydrate the mutation queue from disk. Any offline writes queued
        // before the tab was closed come back as pending here and will be
        // pushed on the next `push()` tick. Without this, `MutationQueue`
        // stayed memory-only and offline mutations were silently lost.
        try {
          const { IndexedDBMutationPersistence } = await import("./persistence");
          const mqPersistence = new IndexedDBMutationPersistence(persistence);
          this.mutations.attachPersistence(mqPersistence);
          await this.mutations.hydrate();
        } catch {
          // Queue persistence optional — memory-only still works.
        }
      } catch {
        // IndexedDB not available — continue without persistence.
      }
    }

    // Seed the server-resolved session before the first pull so
    // `useSession` subscribers see the right tenant from frame one, and
    // `lastSeenTenant` is populated before any subsequent flip can race
    // with it.
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
    // Fires after pull so we don't reconcile against rows that pull
    // would have applied anyway. Errors are swallowed inside
    // reconcileInner so a failed reconcile doesn't take down startup.
    void this.reconcile();

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
      // flips. Pull runs alongside so cursor catches up to anything
      // emitted while the tab was hidden.
      void this.pull();
      void this.reconcile();
    };
    document.addEventListener("visibilitychange", this.visibilityHandler);
  }

  private pollTimer: ReturnType<typeof setInterval> | null = null;

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
    if (this.visibilityHandler && typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", this.visibilityHandler);
      this.visibilityHandler = null;
    }
    this.setConnectionStatus("offline");
  }

  /** Connect to the WebSocket server for real-time updates. */
  private connectWs(): void {
    if (!this.running) return;

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
      // Re-send any active CRDT subscriptions across the new socket.
      // The server purged them on disconnect (`unsubscribe_all`), so
      // without this resync a tab that was subscribed before a network
      // blip would silently stop receiving binary CRDT frames.
      for (const key of this.crdtSubscriptions) {
        const [entity, rowId] = key.split("\x00");
        this.sendWs({ type: "crdt-subscribe", entity, rowId });
      }
      // Re-register every reactive subscription on the fresh socket.
      // The server's ReactiveRegistry tears down on disconnect (via
      // `disconnect_client`) so without this resync the handlers
      // would silently stop receiving result pushes.
      for (const [sub_id, spec] of this.reactiveSpecs) {
        this.sendWs({
          type: "reactive-subscribe",
          sub_id,
          fn_name: spec.fn_name,
          args: spec.args,
        });
      }
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
        for (const handler of this.binaryHandlers) {
          try {
            handler(new Uint8Array(event.data));
          } catch (err) {
            console.warn("[sync] binary handler threw:", err);
          }
        }
        return;
      }

      try {
        const msg = JSON.parse(event.data as string);

        // Sync change event. Persist BEFORE advancing the cursor so a crash
        // can't leave `last_seq` ahead of the replica on disk.
        if (msg.seq && msg.entity && msg.kind) {
          const change = msg as ChangeEvent;
          if (change.seq > this.cursor.last_seq) {
            void this.store.applyChangesAsync([change]).then(async () => {
              this.cursor = { last_seq: change.seq };
              if (this.persistence) {
                await this.persistence.saveCursor(this.cursor);
              }
            });
          }
          return;
        }

        // Presence event.
        if (msg.type === "presence") {
          this.store.notify();
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
        // the handler registered by `subscribeReactive` so the React
        // hook re-renders.
        if (msg.type === "reactive-result" && typeof msg.sub_id === "string") {
          const handler = this.reactiveHandlers.get(msg.sub_id);
          if (handler) {
            handler({ kind: "result", result: msg.result });
          }
          return;
        }
        if (msg.type === "reactive-error" && typeof msg.sub_id === "string") {
          const handler = this.reactiveHandlers.get(msg.sub_id);
          if (handler) {
            handler({
              kind: "error",
              code: typeof msg.code === "string" ? msg.code : "REACTIVE_ERROR",
              message: typeof msg.message === "string" ? msg.message : "",
            });
          }
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
            if (change.seq > this.cursor.last_seq) {
              void this.store.applyChangesAsync([change]).then(async () => {
                this.cursor = { last_seq: change.seq };
                if (this.persistence) {
                  await this.persistence.saveCursor(this.cursor);
                }
              });
            }
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
    const isHttps = url.protocol === "https:";
    const scheme = isHttps ? "wss" : "ws";

    // Production HTTPS deploys: multiplex WS on the same origin via
    // `/api/sync/ws`. The Pylon runtime accepts the Upgrade on its
    // main HTTP port (4321), so any reverse proxy that already
    // forwards `/api/*` carries the WebSocket through too. No
    // separate WS port to expose, no per-deployment wsUrl env var.
    //
    // Local dev (`pylon dev` on `http://localhost:4321`) keeps the
    // legacy port+1 fallback so existing tutorials still work without
    // touching their app config — the dedicated `:4322` listener is
    // still running there too.
    if (url.port) {
      const port = parseInt(url.port, 10);
      return `${scheme}://${url.hostname}:${port + 1}`;
    }
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

  /** Pull changes from the server. */
  async pull(): Promise<void> {
    // Identity change detection. If the token flipped since the last pull
    // (anonymous → signed in, user A → user B, signed in → signed out),
    // the server's visible set changed under us and the cursor we saved
    // reflects the previous identity. Reset before pulling so we rebuild
    // the replica from seq=0 under the new identity.
    const tokenNow = this.currentToken();
    if (
      this.lastSeenToken !== undefined &&
      this.lastSeenToken !== tokenNow
    ) {
      await this.resetReplica();
      // Token flipped → the cached tenant is for the previous user. Pull
      // the fresh session in parallel with the cursor catch-up below.
      void this.refreshResolvedSession();
    }
    this.lastSeenToken = tokenNow;

    try {
      const resp = await this.request<PullResponse>(
        "GET",
        `/api/sync/pull?since=${this.cursor.last_seq}`
      );
      // Successful response — clear the 410 circuit breaker.
      this.consecutive_410s = 0;
      if (resp.changes.length > 0) {
        // Await disk writes before touching the cursor so a crash here can't
        // persist a cursor that's ahead of what actually landed in IndexedDB.
        await this.store.applyChangesAsync(resp.changes);
      }
      // Always advance the cursor to whatever the server reports, not just
      // when changes land. If a read policy filters out every event in a
      // window the server still moves its last_seq forward; clamping to only
      // "non-empty" responses pins the client at `since=0` forever and turns
      // every reconnect into another pull for the same empty window.
      if (resp.cursor && resp.cursor.last_seq > this.cursor.last_seq) {
        this.cursor = resp.cursor;
        if (this.persistence) {
          await this.persistence.saveCursor(this.cursor);
        }
      }
      // If there are more, pull again immediately.
      if (resp.has_more) {
        await this.pull();
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
          await this.resetReplica();
          await this.pull();
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

  /** In-flight reconcile promise — coalesces concurrent callers so a
   *  visibility-change firing during an in-progress reconcile doesn't
   *  double the work. */
  private inFlightReconcile: Promise<void> | null = null;

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
    if (this.inFlightReconcile) return this.inFlightReconcile;
    const minIntervalMs = this.config.reconcileMinIntervalMs ?? 2_000;
    const now = Date.now();
    if (entities === undefined && now - this.lastReconcileAt < minIntervalMs) {
      return;
    }
    const work = this.reconcileInner(entities).finally(() => {
      this.inFlightReconcile = null;
      this.lastReconcileAt = Date.now();
    });
    this.inFlightReconcile = work;
    return work;
  }

  private async reconcileInner(entities?: string[]): Promise<void> {
    const names = entities ?? this.store.entityNames();
    if (names.length === 0) return;
    // Tombstone seq for any local row the server doesn't return. Using
    // the current cursor means future inserts (which have higher seqs)
    // bypass the tombstone — re-creation server-side still propagates.
    const tombstoneSeq = this.cursor.last_seq;
    for (const entity of names) {
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
    const serverIds = new Set<string>();
    const changes: ChangeEvent[] = [];
    for (const row of serverRows) {
      const id = (row as { id?: unknown }).id;
      if (typeof id !== "string" || id.length === 0) continue;
      serverIds.add(id);
      const local = this.store.get(entity, id);
      if (!local) {
        changes.push({
          seq: tombstoneSeq + 1,
          entity,
          row_id: id,
          kind: "insert",
          data: row,
          timestamp: "",
        });
      } else if (rowsDiffer(local, row)) {
        changes.push({
          seq: tombstoneSeq + 1,
          entity,
          row_id: id,
          kind: "update",
          data: row,
          timestamp: "",
        });
      }
    }
    if (changes.length > 0) {
      await this.store.applyChangesAsync(changes);
    }
    // Removals: every local row whose id isn't in the server set is
    // stale. Tombstone with the current cursor so future legitimate
    // re-creations still flow through.
    const locals = this.store.list(entity);
    let removed = false;
    for (const local of locals) {
      const id = (local as { id?: unknown }).id;
      if (typeof id !== "string") continue;
      if (!serverIds.has(id)) {
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
    }
    if (removed) this.store.notify();
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
   * Fetch `/api/auth/me` and update the cached `_resolvedSession`. Callers:
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
    try {
      const res = await this.rawFetch("/api/auth/me");
      if (!res.ok) return;
      const raw = (await res.json()) as {
        user_id?: string | null;
        tenant_id?: string | null;
        is_admin?: boolean;
        roles?: string[];
      };
      const next: ResolvedSession = {
        userId: raw.user_id ?? null,
        tenantId: raw.tenant_id ?? null,
        isAdmin: raw.is_admin ?? false,
        roles: raw.roles ?? [],
      };
      const tenantNow = next.tenantId;
      // First observation seeds lastSeenTenant without a reset — we have
      // nothing to invalidate yet. Subsequent changes flip the replica.
      if (
        this.lastSeenTenant !== undefined &&
        this.lastSeenTenant !== tenantNow
      ) {
        await this.resetReplica();
      }
      this.lastSeenTenant = tenantNow;
      const prev = this._resolvedSession;
      const changed =
        prev.userId !== next.userId ||
        prev.tenantId !== next.tenantId ||
        prev.isAdmin !== next.isAdmin ||
        prev.roles.join(",") !== next.roles.join(",");
      if (changed) {
        this._resolvedSession = next;
        // Piggy-back on the store notifier so `useSession` re-renders via
        // useSyncExternalStore without a second pub/sub channel.
        this.store.notify();
      }
    } catch {
      // Swallow — /api/auth/me errors are transient and the next pull
      // will retry. Don't take down the sync loop for this.
    }
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

  /**
   * In-flight push promise. Used as a mutex so a slow push can't be restarted
   * by the poll timer or a user mutation, which would resend the same batch
   * and cause duplicate writes on the server. The mutation `op_id` keeps
   * that safe at the protocol level (the server deduplicates), but shipping
   * the same batch twice is still wasted bandwidth — hold them instead.
   *
   * Callers always get the SAME promise while a push is running; chain a
   * `.then(() => next push)` if you need a follow-up push after this one.
   */
  private inFlightPush: Promise<void> | null = null;

  /** Push pending mutations to the server. Coalesces concurrent callers. */
  async push(): Promise<void> {
    if (this.inFlightPush) {
      return this.inFlightPush;
    }
    const work = this.pushInner().finally(() => {
      this.inFlightPush = null;
    });
    this.inFlightPush = work;
    return work;
  }

  private async pushInner(): Promise<void> {
    const pending = this.mutations.pending();
    if (pending.length === 0) return;

    try {
      const resp = await this.request<PushResponse>("POST", "/api/sync/push", {
        changes: pending.map((m) => m.change),
        client_id: this.clientId,
      });

      // Mark mutations based on response.
      for (let i = 0; i < pending.length; i++) {
        if (i < resp.applied) {
          this.mutations.markApplied(pending[i].id);
        } else if (resp.errors[i - resp.applied]) {
          this.mutations.markFailed(pending[i].id, resp.errors[i - resp.applied]);
        }
      }

      this.mutations.clear();
    } catch {
      // Will retry on next tick. op_id makes retries idempotent on the server.
    }
  }

  /** Insert a row with optimistic local update. */
  async insert(entity: string, data: Row): Promise<string> {
    const tempId = this.store.optimisticInsert(entity, data);
    this.mutations.add({
      entity,
      row_id: tempId,
      kind: "insert",
      data,
    });
    await this.push();
    return tempId;
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
      this.crdtSubscriptions.add(key);
      this.sendWs({ type: "crdt-subscribe", entity, rowId });
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
      this.crdtSubscriptions.delete(key);
      this.sendWs({ type: "crdt-unsubscribe", entity, rowId });
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
    this.reactiveSpecs.set(sub_id, { fn_name, args });
    this.reactiveHandlers.set(sub_id, handler);
    this.sendWs({ type: "reactive-subscribe", sub_id, fn_name, args });
  }

  /** Tear down a reactive subscription. Sends the unsubscribe to the
   *  server and clears local state. No-op for unknown sub_ids — React
   *  StrictMode double-unmount won't error. */
  unsubscribeReactive(sub_id: string): void {
    if (!this.reactiveSpecs.has(sub_id)) return;
    this.reactiveSpecs.delete(sub_id);
    this.reactiveHandlers.delete(sub_id);
    this.sendWs({ type: "reactive-unsubscribe", sub_id });
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
  const resolved =
    baseUrl ??
    (typeof window !== "undefined" && window.location?.origin
      ? window.location.origin
      : "http://localhost:4321");
  return new SyncEngine({
    ...(options ?? {}),
    baseUrl: resolved,
  });
}
