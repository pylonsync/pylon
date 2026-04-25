// ---------------------------------------------------------------------------
// Storage — synchronous key-value adapter for small, hot pieces of state
// (auth token, client_id). Pluggable so non-browser hosts can swap in their
// own backend without forking the sync engine.
//
// Why synchronous? `connectWs()` and `currentToken()` are called on hot
// paths where awaiting a Promise would force the engine to be async-all-the-
// way-down. The web has localStorage (sync). RN/Tauri/Workers can wrap an
// async backend (AsyncStorage / Tauri-store / KV) by reading the value into
// memory at startup and writing through.
// ---------------------------------------------------------------------------

export interface Storage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
}

// ---------------------------------------------------------------------------
// Default: localStorage when available, in-memory no-op otherwise.
//
// The no-op variant lets the engine boot in SSR, Workers, and Node test
// environments without crashing on `window` references. Tokens won't survive
// process restarts in that mode — callers wanting persistence in those
// environments inject their own adapter.
// ---------------------------------------------------------------------------

class LocalStorageAdapter implements Storage {
  get(key: string): string | null {
    try {
      return window.localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  set(key: string, value: string): void {
    try {
      window.localStorage.setItem(key, value);
    } catch {
      /* quota exceeded, private mode, etc — drop write */
    }
  }
  remove(key: string): void {
    try {
      window.localStorage.removeItem(key);
    } catch {
      /* swallow */
    }
  }
}

class MemoryStorageAdapter implements Storage {
  private map = new Map<string, string>();
  get(key: string): string | null {
    return this.map.get(key) ?? null;
  }
  set(key: string, value: string): void {
    this.map.set(key, value);
  }
  remove(key: string): void {
    this.map.delete(key);
  }
}

/**
 * Pick a default storage adapter for the current host. Returns a real
 * localStorage wrapper in browsers, an in-memory map elsewhere. Apps that
 * need persistence on non-browser hosts (RN, Tauri, Electron) should pass
 * their own adapter via `SyncEngineConfig.storage` and skip this default.
 */
export function defaultStorage(): Storage {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      // Probe — Safari in private mode throws on the first write.
      const probe = "__pylon_probe__";
      window.localStorage.setItem(probe, "1");
      window.localStorage.removeItem(probe);
      return new LocalStorageAdapter();
    }
  } catch {
    /* localStorage exists but is locked down */
  }
  return new MemoryStorageAdapter();
}

/**
 * Build a `Storage` wrapper around an async backend (AsyncStorage,
 * Tauri-store, etc). The host is responsible for hydrating `seed` from
 * the async backend at startup and for persisting writes on its own
 * schedule. The wrapper itself stays synchronous so the engine doesn't
 * change shape per platform.
 *
 * Typical RN wiring:
 * ```ts
 * const seed = await AsyncStorage.multiGet(KEYS).then(toRecord);
 * const storage = createWriteThroughStorage(seed, async (k, v) => {
 *   if (v === null) await AsyncStorage.removeItem(k);
 *   else await AsyncStorage.setItem(k, v);
 * });
 * init({ baseUrl, storage });
 * ```
 */
export function createWriteThroughStorage(
  seed: Record<string, string> | null,
  onWrite: (key: string, value: string | null) => void,
): Storage {
  const map = new Map<string, string>(Object.entries(seed ?? {}));
  return {
    get: (k) => map.get(k) ?? null,
    set: (k, v) => {
      map.set(k, v);
      onWrite(k, v);
    },
    remove: (k) => {
      map.delete(k);
      onWrite(k, null);
    },
  };
}
