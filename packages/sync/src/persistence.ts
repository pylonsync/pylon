import type {
  Row,
  ChangeEvent,
  SyncCursor,
  MutationQueuePersistence,
  PendingMutation,
} from "./index";

// ---------------------------------------------------------------------------
// IndexedDB persistence layer
// ---------------------------------------------------------------------------

// Bumped DB_VERSION: version 2 adds a pendingMutations object store so the
// offline queue survives restarts. The upgrade handler below creates it on
// existing databases — users never lose their entity mirror.
const DB_NAME = "statecraft_sync";
const DB_VERSION = 2;
const STORE_NAME = "entities";
const CURSOR_STORE = "cursors";
const MUTATIONS_STORE = "pendingMutations";

/**
 * IndexedDB-backed persistence for the sync store.
 * Saves entity rows and sync cursor so data survives page refresh.
 */
export class IndexedDBPersistence {
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

  /** Save a row to IndexedDB. */
  async saveRow(entity: string, id: string, data: Row): Promise<void> {
    if (!this.db) return;
    const tx = this.db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    store.put({ _key: `${entity}:${id}`, entity, id, data });
    return new Promise((resolve) => {
      tx.oncomplete = () => resolve();
    });
  }

  /** Delete a row from IndexedDB. */
  async deleteRow(entity: string, id: string): Promise<void> {
    if (!this.db) return;
    const tx = this.db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    store.delete(`${entity}:${id}`);
    return new Promise((resolve) => {
      tx.oncomplete = () => resolve();
    });
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

  /** Save the sync cursor. */
  async saveCursor(cursor: SyncCursor): Promise<void> {
    if (!this.db) return;
    const tx = this.db.transaction(CURSOR_STORE, "readwrite");
    const store = tx.objectStore(CURSOR_STORE);
    store.put({ key: "cursor", ...cursor });
    return new Promise((resolve) => {
      tx.oncomplete = () => resolve();
    });
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

  /** Clear all stored data. */
  async clear(): Promise<void> {
    if (!this.db) return;
    const tx = this.db.transaction([STORE_NAME, CURSOR_STORE], "readwrite");
    tx.objectStore(STORE_NAME).clear();
    tx.objectStore(CURSOR_STORE).clear();
    return new Promise((resolve) => {
      tx.oncomplete = () => resolve();
    });
  }
}

/**
 * Apply a change event to IndexedDB persistence.
 */
export async function persistChange(
  persistence: IndexedDBPersistence,
  change: ChangeEvent
): Promise<void> {
  switch (change.kind) {
    case "insert":
    case "update":
      if (change.data) {
        await persistence.saveRow(change.entity, change.row_id, change.data);
      }
      break;
    case "delete":
      await persistence.deleteRow(change.entity, change.row_id);
      break;
  }
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
