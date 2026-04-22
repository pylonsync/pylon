import AsyncStorage from "@react-native-async-storage/async-storage";

// ---------------------------------------------------------------------------
// Persistence adapter interface
// ---------------------------------------------------------------------------

/**
 * A simple key-value persistence interface. The default implementation uses
 * React Native's AsyncStorage, but consumers can swap in any backend (e.g.
 * MMKV, expo-secure-store) by implementing this interface.
 */
export interface PersistenceAdapter {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  remove(key: string): Promise<void>;
  keys(): Promise<string[]>;
}

// ---------------------------------------------------------------------------
// AsyncStorage implementation
// ---------------------------------------------------------------------------

/**
 * Default persistence adapter backed by `@react-native-async-storage/async-storage`.
 * All keys are namespaced under a configurable prefix to avoid collisions with
 * other libraries sharing the same storage.
 */
export class AsyncStoragePersistence implements PersistenceAdapter {
  private prefix: string;

  constructor(prefix = "statecraft") {
    this.prefix = prefix;
  }

  private prefixedKey(key: string): string {
    return `${this.prefix}:${key}`;
  }

  async get(key: string): Promise<string | null> {
    return AsyncStorage.getItem(this.prefixedKey(key));
  }

  async set(key: string, value: string): Promise<void> {
    await AsyncStorage.setItem(this.prefixedKey(key), value);
  }

  async remove(key: string): Promise<void> {
    await AsyncStorage.removeItem(this.prefixedKey(key));
  }

  async keys(): Promise<string[]> {
    const allKeys = await AsyncStorage.getAllKeys();
    const prefixWithSep = `${this.prefix}:`;
    return allKeys
      .filter((k) => k.startsWith(prefixWithSep))
      .map((k) => k.slice(prefixWithSep.length));
  }
}

// ---------------------------------------------------------------------------
// Offline store
// ---------------------------------------------------------------------------

/**
 * Higher-level offline store that persists entity rows and sync cursors
 * locally so the app can hydrate instantly on cold start and work offline.
 *
 * ```ts
 * const store = new OfflineStore();
 * await store.saveEntities("Todo", todos);
 * const cached = await store.loadEntities("Todo");
 * ```
 */
export class OfflineStore {
  private adapter: PersistenceAdapter;

  constructor(adapter?: PersistenceAdapter) {
    this.adapter = adapter ?? new AsyncStoragePersistence();
  }

  /** Persist a full entity list for offline access. */
  async saveEntities(
    entity: string,
    rows: Record<string, unknown>[],
  ): Promise<void> {
    await this.adapter.set(`entities:${entity}`, JSON.stringify(rows));
  }

  /** Load a previously persisted entity list. Returns `[]` if nothing cached. */
  async loadEntities(
    entity: string,
  ): Promise<Record<string, unknown>[]> {
    const data = await this.adapter.get(`entities:${entity}`);
    return data ? (JSON.parse(data) as Record<string, unknown>[]) : [];
  }

  /** Save the sync cursor so incremental pulls resume where they left off. */
  async saveCursor(cursor: string): Promise<void> {
    await this.adapter.set("sync:cursor", cursor);
  }

  /** Load the last saved sync cursor. */
  async loadCursor(): Promise<string | null> {
    return this.adapter.get("sync:cursor");
  }

  /** Remove all data managed by this store. */
  async clear(): Promise<void> {
    const keys = await this.adapter.keys();
    await Promise.all(keys.map((key) => this.adapter.remove(key)));
  }
}
