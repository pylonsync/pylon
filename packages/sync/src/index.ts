// ---------------------------------------------------------------------------
// pylon sync client — local-first sync engine
// ---------------------------------------------------------------------------

export { IndexedDBPersistence, persistChange } from "./persistence";

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
}

/**
 * Generate a stable client_id. Prefers a persisted id from localStorage
 * (so a reload keeps the same identifier) and falls back to a fresh UUID.
 * Not persisted when there's no window/localStorage (SSR, workers).
 */
function generateClientId(): string {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      const key = "pylon:client_id";
      const existing = window.localStorage.getItem(key);
      if (existing) return existing;
      const fresh = newUuidLike();
      window.localStorage.setItem(key, fresh);
      return fresh;
    }
  } catch {
    // localStorage disabled / quota exceeded — fall through.
  }
  return newUuidLike();
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

export class SyncEngine {
  private config: SyncEngineConfig;
  private cursor: SyncCursor = { last_seq: 0 };
  private running = false;
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
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

  /** Read the cached resolved session. Null user = anonymous. */
  resolvedSession(): ResolvedSession {
    return this._resolvedSession;
  }

  constructor(config: SyncEngineConfig) {
    this.config = config;
    this.store = new LocalStore();
    this.mutations = new MutationQueue();
    this.clientId = generateClientId();
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
          // @ts-expect-error — reach into the private field to swap in the
          // backend post-construction. Same package, acceptable coupling.
          this.mutations.persistence = mqPersistence;
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

    const transport = this.config.transport ?? "websocket";
    if (transport === "websocket") {
      this.connectWs();
    } else if (transport === "sse") {
      this.connectSse();
    } else if (transport === "poll") {
      this.startPolling();
    }
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
      (typeof window !== "undefined" && window.localStorage
        ? window.localStorage.getItem(this.tokenStorageKey()) ?? undefined
        : undefined);
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
      if (this.wsStableTimer) clearTimeout(this.wsStableTimer);
      this.wsStableTimer = setTimeout(() => {
        this.reconnectAttempts = 0;
        this.wsStableTimer = null;
      }, 5_000);
    };

    this.ws.onmessage = (event) => {
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

    // HTTPS deploys (Fly/Vercel/Cloudflare) terminate TLS at a single
    // public port — we can't assume port+1 is exposed. Callers should
    // override via `wsUrl` in the sync-engine config (or set
    // VITE_PYLON_WS_URL in Vite apps) when the WebSocket listens on a
    // different hostname or a separate Fly service.
    //
    // If the base URL has an explicit port (e.g. http://localhost:4321)
    // we keep the historical port+1 convention — that's what `pylon dev`
    // hands to the developer on a single box. Otherwise we assume the
    // WebSocket is reachable at the same hostname on the same scheme
    // (most production proxies multiplex WS on 443 via the Upgrade
    // header, and a future pylon build will do the same).
    if (url.port) {
      const port = parseInt(url.port, 10);
      return `${scheme}://${url.hostname}:${port + 1}`;
    }
    return `${scheme}://${url.hostname}`;
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
   */
  async resetReplica(): Promise<void> {
    this.cursor = { last_seq: 0 };
    this.store.clearAll();
    if (this.persistence) {
      try {
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

  /** Current auth token from config or localStorage. Null when neither has one. */
  private currentToken(): string | null {
    if (this.config.token) return this.config.token;
    if (typeof window === "undefined" || !window.localStorage) return null;
    return window.localStorage.getItem(this.tokenStorageKey());
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
      if (status === 410) {
        await this.resetReplica();
        // Re-pull immediately; the catch block will swallow nested failures.
        await this.pull();
      }
    }
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
    return fetch(`${this.config.baseUrl}${path}`, { headers });
  }

  /**
   * Public alias for `refreshResolvedSession`. Call after anything that
   * mutates the server session (sign-in, sign-out, `/api/auth/select-org`)
   * so the cached session and React subscribers pick up the change without
   * waiting for the next pull.
   */
  notifySessionChanged(): Promise<void> {
    return this.refreshResolvedSession();
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
      (typeof window !== "undefined" && window.localStorage
        ? window.localStorage.getItem(this.tokenStorageKey()) ?? undefined
        : undefined);
    if (token) headers["Authorization"] = `Bearer ${token}`;

    const res = await fetch(`${this.config.baseUrl}${path}`, {
      method,
      headers,
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

// ---------------------------------------------------------------------------
// Convenience factory
// ---------------------------------------------------------------------------

/** Create a sync engine connected to the pylon dev server. */
export function createSyncEngine(
  baseUrl = "http://localhost:4321",
  options?: Partial<SyncEngineConfig>,
): SyncEngine {
  return new SyncEngine({
    ...(options ?? {}),
    baseUrl,
  });
}
