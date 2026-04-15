import type { Row, ChangeEvent, SyncCursor } from "./index";

// ---------------------------------------------------------------------------
// IndexedDB persistence layer
// ---------------------------------------------------------------------------

const DB_NAME = "agentdb_sync";
const DB_VERSION = 1;
const STORE_NAME = "entities";
const CURSOR_STORE = "cursors";

/**
 * IndexedDB-backed persistence for the sync store.
 * Saves entity rows and sync cursor so data survives page refresh.
 */
export class IndexedDBPersistence {
  private db: IDBDatabase | null = null;
  private dbName: string;

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
      };

      request.onsuccess = () => {
        this.db = request.result;
        resolve();
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
