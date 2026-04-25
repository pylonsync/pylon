import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  createWriteThroughStorage,
  type Storage as PylonStorage,
} from "@pylonsync/sync";

// ---------------------------------------------------------------------------
// Sync key-value adapter (AsyncStorage-backed)
//
// Pylon's sync engine uses a *synchronous* Storage interface for hot-path
// state (auth token, client_id) — async would force the engine to be
// async-all-the-way-down. AsyncStorage is async. The bridge: read seeds
// into memory once at startup, then write through to AsyncStorage in the
// background. Reads are immediate, writes are eventually-consistent.
// ---------------------------------------------------------------------------

/**
 * Hot-path keys we eagerly read into memory at boot. Anything outside this
 * set is missed until the engine writes to it (which then immediately
 * mirrors to AsyncStorage). Add to this list if a new sync-engine-internal
 * key shows up.
 */
const HOT_KEYS = [
  "pylon:client_id",
  "pylon_token",
  // Wildcards aren't supported here; namespace-aware pylon apps register
  // keys via `init({ appName })` before the first request, which prefixes
  // tokens as `pylon:<app>:token`. The seed loader scans for matches.
];

/**
 * Build a sync-engine-compatible Storage adapter on top of AsyncStorage.
 * Reads return immediately from an in-memory cache seeded from AsyncStorage.
 * Writes update the cache and fire-and-forget the AsyncStorage write.
 *
 * Call before `init()` so the SyncEngine constructor can read the cached
 * client_id / token without a Promise hop.
 */
export async function createAsyncStorageBridge(): Promise<PylonStorage> {
  // Pull in any existing pylon keys so a cold launch re-uses the same
  // client_id (so the server's per-client diagnostics line up across
  // sessions) and the same token (so the user stays signed in).
  const allKeys = await AsyncStorage.getAllKeys();
  const pylonKeys = allKeys.filter(
    (k) => k.startsWith("pylon_") || k.startsWith("pylon:"),
  );
  const merged = [...new Set([...HOT_KEYS, ...pylonKeys])];
  const seed: Record<string, string> = {};
  if (merged.length > 0) {
    const values = await Promise.all(merged.map((k) => AsyncStorage.getItem(k)));
    for (let i = 0; i < merged.length; i++) {
      const v = values[i];
      if (v != null) seed[merged[i]] = v;
    }
  }
  return createWriteThroughStorage(seed, (key, value) => {
    if (value === null) {
      void AsyncStorage.removeItem(key);
    } else {
      void AsyncStorage.setItem(key, value);
    }
  });
}

// ---------------------------------------------------------------------------
// Optional higher-level offline-row store
//
// Kept for apps that want a separate manual cache (e.g. offline mutation
// queue persistence outside the sync engine's IndexedDB equivalent).
// SyncEngine handles the common case automatically once given a Storage.
// ---------------------------------------------------------------------------

export interface PersistenceAdapter {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  remove(key: string): Promise<void>;
  keys(): Promise<string[]>;
}

export class AsyncStoragePersistence implements PersistenceAdapter {
  private prefix: string;

  constructor(prefix = "pylon") {
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

export class OfflineStore {
  private adapter: PersistenceAdapter;

  constructor(adapter?: PersistenceAdapter) {
    this.adapter = adapter ?? new AsyncStoragePersistence();
  }

  async saveEntities(
    entity: string,
    rows: Record<string, unknown>[],
  ): Promise<void> {
    await this.adapter.set(`entities:${entity}`, JSON.stringify(rows));
  }

  async loadEntities(
    entity: string,
  ): Promise<Record<string, unknown>[]> {
    const data = await this.adapter.get(`entities:${entity}`);
    return data ? (JSON.parse(data) as Record<string, unknown>[]) : [];
  }

  async saveCursor(cursor: string): Promise<void> {
    await this.adapter.set("sync:cursor", cursor);
  }

  async loadCursor(): Promise<string | null> {
    return this.adapter.get("sync:cursor");
  }

  async clear(): Promise<void> {
    const keys = await this.adapter.keys();
    await Promise.all(keys.map((key) => this.adapter.remove(key)));
  }
}
