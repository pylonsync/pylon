import type {
  Row,
  ChangeEvent,
  SyncCursor,
  MutationQueuePersistence,
  PendingMutation,
} from "./index";

// ---------------------------------------------------------------------------
// Replica persistence contract
// ---------------------------------------------------------------------------

/**
 * The engine's durable-replica backend. IndexedDBPersistence is the browser
 * implementation; non-browser hosts (React Native, Tauri) implement this
 * and pass it via `SyncEngineConfig.persistence` — the exact call surface
 * the engine + `persistChange` use, nothing more.
 */
export interface ReplicaPersistence {
  open(): Promise<void>;
  /** Rows + cursor read as ONE consistent snapshot (see warm-load notes). */
  loadSnapshot(): Promise<{
    entities: Record<string, Row[]>;
    cursor: SyncCursor | null;
    hadCache: boolean;
  }>;
  /** `undefined` = never recorded (fresh install); `null` = anonymous. */
  loadIdentity(): Promise<string | null | undefined>;
  saveIdentity(userId: string | null): Promise<boolean>;
  saveCursor(cursor: SyncCursor): Promise<boolean>;
  saveRow(entity: string, id: string, data: Row): Promise<boolean>;
  deleteRow(entity: string, id: string): Promise<boolean>;
  clear(): Promise<boolean>;
}

// ---------------------------------------------------------------------------
// IndexedDB persistence layer
// ---------------------------------------------------------------------------

// Bumped DB_VERSION: version 2 adds a pendingMutations object store so the
// offline queue survives restarts. The upgrade handler below creates it on
// existing databases — users never lose their entity mirror.
const DB_NAME = "pylon_sync";
const DB_VERSION = 2;
const STORE_NAME = "entities";
const CURSOR_STORE = "cursors";
const MUTATIONS_STORE = "pendingMutations";

/**
 * IndexedDB-backed persistence for the sync store.
 * Saves entity rows and sync cursor so data survives page refresh.
 */
export class IndexedDBPersistence implements ReplicaPersistence {
  private db: IDBDatabase | null = null;
  private dbName: string;

  /** Shared connection. Exposed so sibling persistence classes (e.g. the
   *  mutation-queue backend) can reuse the same IDBDatabase — IndexedDB only
   *  permits one open handle per (origin, db) at a time while upgrades run.
   */
  get connection(): IDBDatabase | null {
    return this.db;
  }

  constructor(appName = "default") {
    this.dbName = `${DB_NAME}_${appName}`;
  }

  async open(): Promise<void> {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(this.dbName, DB_VERSION);

      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: "_key" });
        }
        if (!db.objectStoreNames.contains(CURSOR_STORE)) {
          db.createObjectStore(CURSOR_STORE, { keyPath: "key" });
        }
        // v2: durable offline mutation queue.
        if (!db.objectStoreNames.contains(MUTATIONS_STORE)) {
          db.createObjectStore(MUTATIONS_STORE, { keyPath: "id" });
        }
      };

      request.onsuccess = () => {
        this.db = request.result;
        // If another tab later bumps the version, the browser fires
        // `versionchange` on this handle. Close it so the other tab's
        // upgrade can proceed — otherwise THEIR start() hangs on our
        // stale handle. Our app will see the underlying reads fail and
        // degrade to memory-only gracefully.
        this.db.onversionchange = () => {
          this.db?.close();
          this.db = null;
        };
        resolve();
      };

      // `onblocked` fires when we try to upgrade but another tab holds
      // an older-version connection open. Rejecting here (rather than
      // waiting forever) lets `start()` fall back to memory-only mode,
      // which is still functional for the current session. The next tab
      // reload after the other tab closes will pick up the new version.
      request.onblocked = () => {
        reject(new Error("IndexedDB upgrade blocked by another open connection"));
      };

      request.onerror = () => {
        reject(new Error("Failed to open IndexedDB"));
      };
    });
  }

  /** Resolve when a write transaction settles, reporting durability:
   *  `true` on COMMIT, `false` if it aborts/errors — e.g.
   *  QuotaExceededError on a storage-pressured device. NEVER rejects or
   *  hangs: registering only `oncomplete` (the original shape) meant an
   *  aborted tx never settled, and the engine awaits the persist before
   *  advancing the cursor in `enqueueApply` — so ONE hung write wedged
   *  the whole applyQueue and live sync silently died for the session.
   *
   *  But "resolve unconditionally" (the first fix) was also wrong: on an
   *  abort the row never reached disk, yet `enqueueApply` still advanced
   *  the PERSISTED cursor past it — so on restart the warm-load skipped
   *  those rows forever (cursor ahead of replica). The boolean lets the
   *  caller hold the on-disk cursor back when a row write fails, so the
   *  next cold start re-pulls the gap. Best-effort: log once, keep the
   *  in-memory replica authoritative, but never persist a cursor ahead of
   *  what's actually durable. Mirrors the read paths (`getRow`/
   *  `loadCursor`) which already degrade via `onerror`. */
  private commit(tx: IDBTransaction, op: string): Promise<boolean> {
    return new Promise((resolve) => {
      tx.oncomplete = () => resolve(true);
      tx.onerror = () => {
        // eslint-disable-next-line no-console
        console.warn(`[pylon-sync] IndexedDB ${op} failed; degrading to memory`, tx.error);
        resolve(false);
      };
      tx.onabort = () => {
        // eslint-disable-next-line no-console
        console.warn(`[pylon-sync] IndexedDB ${op} aborted; degrading to memory`, tx.error);
        resolve(false);
      };
    });
  }

  /** Save a row to IndexedDB. Resolves `true` when the write is durable,
   *  `false` if it degraded (no DB / aborted) — see `commit`. */
  async saveRow(entity: string, id: string, data: Row): Promise<boolean> {
    if (!this.db) return false;
    const tx = this.db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    store.put({ _key: `${entity}:${id}`, entity, id, data });
    return this.commit(tx, "saveRow");
  }

  /** Fetch a row from IndexedDB by key. Used by `persistChange` on update
   *  events to merge the patch against what's already on disk. */
  async getRow(entity: string, id: string): Promise<Row | null> {
    if (!this.db) return null;
    const tx = this.db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const request = store.get(`${entity}:${id}`);
    return new Promise((resolve) => {
      request.onsuccess = () => {
        const rec = request.result as { data?: Row } | undefined;
        resolve(rec?.data ?? null);
      };
      request.onerror = () => resolve(null);
    });
  }

  /** Delete a row from IndexedDB. Resolves `true` when durable, `false`
   *  if it degraded — see `commit`. */
  async deleteRow(entity: string, id: string): Promise<boolean> {
    if (!this.db) return false;
    const tx = this.db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    store.delete(`${entity}:${id}`);
    return this.commit(tx, "deleteRow");
  }

  /** Load all rows for an entity from IndexedDB. */
  async loadAll(entity: string): Promise<Row[]> {
    if (!this.db) return [];
    const tx = this.db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const request = store.getAll();

    return new Promise((resolve) => {
      request.onsuccess = () => {
        const rows = (request.result as { entity: string; id: string; data: Row }[])
          .filter((r) => r.entity === entity)
          .map((r) => ({ id: r.id, ...r.data }));
        resolve(rows);
      };
      request.onerror = () => resolve([]);
    });
  }

  /** Load all entities and their rows from IndexedDB. */
  async loadAllEntities(): Promise<Record<string, Row[]>> {
    if (!this.db) return {};
    const tx = this.db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const request = store.getAll();

    return new Promise((resolve) => {
      request.onsuccess = () => {
        const result: Record<string, Row[]> = {};
        for (const item of request.result as { entity: string; id: string; data: Row }[]) {
          if (!result[item.entity]) result[item.entity] = [];
          result[item.entity].push({ id: item.id, ...item.data });
        }
        resolve(result);
      };
      request.onerror = () => resolve({});
    });
  }

  /**
   * Atomic warm-load: returns entities + cursor in a single IDB
   * read transaction. Used by `SyncEngine.start()` to hydrate the
   * in-memory replica BEFORE the network pull resolves so React
   * hooks see real data on first render (no empty-then-populated
   * flash on returning visits).
   *
   * `hadCache` is true when at least one row OR a saved cursor
   * was found. The engine uses it to distinguish "true cold start
   * — pull-from-0 IS a full snapshot, skip the post-snapshot
   * reconcile" from "returning user with cached state — pull-from-
   * cursor may miss server-side deletes that happened offline, the
   * onConnected reconcile MUST run". Without that distinction, a
   * returning user whose cursor somehow rolled back to 0 (rare:
   * IDB partial corruption, cleared-by-mistake) would end up with
   * ghost rows that survive forever.
   *
   * Single readonly tx is intentional — two separate reads could
   * race a mid-load saveCursor/saveRow from another tab's apply
   * pipeline and read an inconsistent (cursor C', rows for cursor C)
   * pair. One tx guarantees a consistent snapshot.
   */
  async loadSnapshot(): Promise<{
    entities: Record<string, Row[]>;
    cursor: SyncCursor | null;
    hadCache: boolean;
  }> {
    if (!this.db) return { entities: {}, cursor: null, hadCache: false };
    const tx = this.db.transaction([STORE_NAME, CURSOR_STORE], "readonly");
    const entitiesReq = tx.objectStore(STORE_NAME).getAll();
    const cursorReq = tx.objectStore(CURSOR_STORE).get("cursor");
    return new Promise((resolve) => {
      tx.oncomplete = () => {
        const entities: Record<string, Row[]> = {};
        for (const item of (entitiesReq.result ?? []) as {
          entity: string;
          id: string;
          data: Row;
        }[]) {
          if (!entities[item.entity]) entities[item.entity] = [];
          entities[item.entity].push({ id: item.id, ...item.data });
        }
        const cursorRec = cursorReq.result as { last_seq?: number } | undefined;
        const cursor: SyncCursor | null = cursorRec
          ? { last_seq: cursorRec.last_seq ?? 0 }
          : null;
        const hadCache =
          Object.keys(entities).length > 0 || cursor !== null;
        resolve({ entities, cursor, hadCache });
      };
      tx.onerror = () => resolve({ entities: {}, cursor: null, hadCache: false });
      tx.onabort = () => resolve({ entities: {}, cursor: null, hadCache: false });
    });
  }

  /** Save the sync cursor. Resolves `true` when durable, `false` if it
   *  degraded — see `commit`. */
  async saveCursor(cursor: SyncCursor): Promise<boolean> {
    if (!this.db) return false;
    const tx = this.db.transaction(CURSOR_STORE, "readwrite");
    const store = tx.objectStore(CURSOR_STORE);
    store.put({ key: "cursor", ...cursor });
    return this.commit(tx, "saveCursor");
  }

  /** Load the sync cursor. */
  async loadCursor(): Promise<SyncCursor | null> {
    if (!this.db) return null;
    const tx = this.db.transaction(CURSOR_STORE, "readonly");
    const store = tx.objectStore(CURSOR_STORE);
    const request = store.get("cursor");

    return new Promise((resolve) => {
      request.onsuccess = () => {
        if (request.result) {
          resolve({ last_seq: request.result.last_seq ?? 0 });
        } else {
          resolve(null);
        }
      };
      request.onerror = () => resolve(null);
    });
  }

  /** Record which identity (resolved `userId`, or `null` when logged out)
   *  the persisted replica belongs to. Read back on the next cold start to
   *  detect an account switch across a page reload — the one case the live
   *  `observeToken` path can't see (a fresh engine has no prior token to
   *  compare against). Stored in the cursor store so it rides the same DB. */
  async saveIdentity(userId: string | null): Promise<boolean> {
    if (!this.db) return false;
    const tx = this.db.transaction(CURSOR_STORE, "readwrite");
    tx.objectStore(CURSOR_STORE).put({ key: "identity", userId });
    return this.commit(tx, "saveIdentity");
  }

  /** Load the persisted replica identity. Returns `undefined` when no tag
   *  was ever written (a pre-tag replica — the caller must not treat that
   *  as a mismatch), or `string | null` for a recorded identity. */
  async loadIdentity(): Promise<string | null | undefined> {
    if (!this.db) return undefined;
    const tx = this.db.transaction(CURSOR_STORE, "readonly");
    const store = tx.objectStore(CURSOR_STORE);
    const request = store.get("identity");
    return new Promise((resolve) => {
      request.onsuccess = () => {
        resolve(request.result ? (request.result.userId ?? null) : undefined);
      };
      request.onerror = () => resolve(undefined);
    });
  }

  /** Clear stored rows + cursor. Deliberately does NOT touch the
   *  durable mutation queue: `resetReplica` calls this on a 410 RESYNC
   *  (same user, needs a fresh snapshot) where pending offline writes
   *  MUST survive. On an IDENTITY flip the old identity's pending
   *  mutations must instead be discarded — that wipe runs through
   *  `MutationQueue.clearAll()` (which persists an empty `MUTATIONS_STORE`
   *  via its own backend), gated on `resetReplica({ wipeMutations })` at
   *  the token/tenant-flip call sites. Splitting the two stores keeps each
   *  reset path honest: 410 keeps writes, identity flip drops them. */
  async clear(): Promise<boolean> {
    if (!this.db) return false;
    const tx = this.db.transaction([STORE_NAME, CURSOR_STORE], "readwrite");
    tx.objectStore(STORE_NAME).clear();
    tx.objectStore(CURSOR_STORE).clear();
    return this.commit(tx, "clear");
  }
}

/**
 * Apply a change event to IndexedDB persistence. Returns `true` when the
 * write reached disk durably, `false` when it degraded (so `enqueueApply`
 * can hold the persisted cursor back rather than skip the row on restart).
 * A change with no `data` is a no-op and counts as durable.
 */
export async function persistChange(
  persistence: ReplicaPersistence,
  change: ChangeEvent
): Promise<boolean> {
  switch (change.kind) {
    case "insert":
    case "update":
      if (change.data) {
        // Callers upstream (LocalStore.applyChanges/Async) hydrate the
        // change's `data` with the post-merge row from memory — so even
        // on update the full row lands here. Overwriting is correct;
        // pre-merge patches would have dropped unpatched columns.
        return persistence.saveRow(change.entity, change.row_id, change.data);
      }
      return true;
    case "delete":
      return persistence.deleteRow(change.entity, change.row_id);
  }
  return true;
}

// ---------------------------------------------------------------------------
// Mutation-queue persistence (the durable offline write buffer)
// ---------------------------------------------------------------------------

/**
 * IndexedDB-backed implementation of `MutationQueuePersistence`. Wires the
 * `MutationQueue` into the same database as the entity mirror so everything
 * the app needs to resume a session lives in one place.
 *
 * `saveAll` writes the entire queue on every change. That's O(n) per write,
 * but `n` is bounded by "how many mutations the user queued while offline",
 * which is tiny in practice. If that ever becomes a bottleneck, switch to
 * per-id `put`/`delete` — the schema (`keyPath: "id"`) already supports it.
 */
export class IndexedDBMutationPersistence implements MutationQueuePersistence {
  private db: IDBDatabase | null = null;

  constructor(private readonly parent: IndexedDBPersistence) {}

  private handle(): IDBDatabase | null {
    return this.parent.connection;
  }

  async saveAll(mutations: PendingMutation[]): Promise<void> {
    const db = this.handle();
    if (!db) return;
    const tx = db.transaction(MUTATIONS_STORE, "readwrite");
    const store = tx.objectStore(MUTATIONS_STORE);
    // Clear then re-put everything. Simpler than diffing, and correct under
    // the "save-full-snapshot on every change" contract.
    store.clear();
    for (const m of mutations) {
      store.put(m);
    }
    return new Promise((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("mutation queue save failed"));
      tx.onabort = () => reject(tx.error ?? new Error("mutation queue save aborted"));
    });
  }

  async loadAll(): Promise<PendingMutation[]> {
    const db = this.handle();
    if (!db) return [];
    const tx = db.transaction(MUTATIONS_STORE, "readonly");
    const store = tx.objectStore(MUTATIONS_STORE);
    return new Promise((resolve, reject) => {
      const req = store.getAll();
      req.onsuccess = () => resolve((req.result as PendingMutation[]) ?? []);
      req.onerror = () => reject(req.error ?? new Error("mutation queue load failed"));
    });
  }
}
