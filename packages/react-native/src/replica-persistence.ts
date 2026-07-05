import AsyncStorage from "@react-native-async-storage/async-storage";
import type { ReplicaPersistence, Row, SyncCursor } from "@pylonsync/sync";

// ---------------------------------------------------------------------------
// AsyncStorage-backed replica persistence
//
// The engine's durable replica on React Native. Without this, an RN app's
// replica was memory-only: a cold launch with no connectivity (the gym
// basement — the core Spotter-class use case) rendered an empty store
// until the network came back.
//
// Layout: one JSON blob per entity (`<prefix>:e:<Entity>`) plus cursor +
// identity keys. Writes go through an in-memory mirror with per-entity
// coalescing: a burst of `saveRow`s (a pull page applying) collapses into
// one AsyncStorage write per entity per tick, and callers' promises
// resolve with that write's real durability — the engine holds the
// persisted cursor back when a row write fails, so a crash can never
// leave the cursor ahead of what's actually on disk (the same contract
// the IndexedDB backend documents).
// ---------------------------------------------------------------------------

const CURSOR_KEY = "cursor";
const IDENTITY_KEY = "identity";

export class AsyncStorageReplicaPersistence implements ReplicaPersistence {
  private prefix: string;
  private mirror = new Map<string, Map<string, Row>>();
  private pendingFlush = new Map<string, Promise<boolean>>();
  private loaded = false;

  constructor(appName = "default") {
    this.prefix = `pylon:${appName}:replica`;
  }

  private key(suffix: string): string {
    return `${this.prefix}:${suffix}`;
  }

  async open(): Promise<void> {
    if (this.loaded) return;
    const allKeys = await AsyncStorage.getAllKeys();
    const entityKeys = allKeys.filter((k) => k.startsWith(this.key("e:")));
    if (entityKeys.length > 0) {
      // Only the surface every async-storage major ships: per-key gets.
      const values = await Promise.all(entityKeys.map((k) => AsyncStorage.getItem(k)));
      for (let i = 0; i < entityKeys.length; i++) {
        const k = entityKeys[i];
        const v = values[i];
        if (!v) continue;
        const entity = k.slice(this.key("e:").length);
        try {
          const rows = JSON.parse(v) as Row[];
          const m = new Map<string, Row>();
          for (const r of rows) {
            const id = r.id;
            if (typeof id === "string") m.set(id, r);
          }
          this.mirror.set(entity, m);
        } catch {
          // A corrupt blob loses that entity's cache, not the session.
        }
      }
    }
    this.loaded = true;
  }

  async loadSnapshot(): Promise<{
    entities: Record<string, Row[]>;
    cursor: SyncCursor | null;
    hadCache: boolean;
  }> {
    await this.open();
    const entities: Record<string, Row[]> = {};
    for (const [entity, rows] of this.mirror) {
      entities[entity] = [...rows.values()];
    }
    let cursor: SyncCursor | null = null;
    const rawCursor = await AsyncStorage.getItem(this.key(CURSOR_KEY));
    if (rawCursor) {
      try {
        cursor = JSON.parse(rawCursor) as SyncCursor;
      } catch {
        cursor = null;
      }
    }
    return {
      entities,
      cursor,
      hadCache: this.mirror.size > 0 || cursor != null,
    };
  }

  async loadIdentity(): Promise<string | null | undefined> {
    const raw = await AsyncStorage.getItem(this.key(IDENTITY_KEY));
    if (raw == null) return undefined; // never recorded
    return raw === "" ? null : raw; // "" encodes anonymous
  }

  async saveIdentity(userId: string | null): Promise<boolean> {
    try {
      await AsyncStorage.setItem(this.key(IDENTITY_KEY), userId ?? "");
      return true;
    } catch {
      return false;
    }
  }

  async saveCursor(cursor: SyncCursor): Promise<boolean> {
    try {
      await AsyncStorage.setItem(this.key(CURSOR_KEY), JSON.stringify(cursor));
      return true;
    } catch {
      return false;
    }
  }

  async saveRow(entity: string, id: string, data: Row): Promise<boolean> {
    let m = this.mirror.get(entity);
    if (!m) {
      m = new Map();
      this.mirror.set(entity, m);
    }
    m.set(id, data);
    return this.flushEntity(entity);
  }

  async deleteRow(entity: string, id: string): Promise<boolean> {
    const m = this.mirror.get(entity);
    if (m) m.delete(id);
    return this.flushEntity(entity);
  }

  /** Coalesce a same-tick burst of writes into one AsyncStorage set per
   * entity; every caller in the burst shares the write's durability. */
  private flushEntity(entity: string): Promise<boolean> {
    const existing = this.pendingFlush.get(entity);
    if (existing) return existing;
    const p = (async () => {
      await Promise.resolve(); // let the burst finish mutating the mirror
      this.pendingFlush.delete(entity);
      const m = this.mirror.get(entity);
      try {
        await AsyncStorage.setItem(
          this.key(`e:${entity}`),
          JSON.stringify(m ? [...m.values()] : []),
        );
        return true;
      } catch {
        return false; // quota/etc — engine holds the cursor back
      }
    })();
    this.pendingFlush.set(entity, p);
    return p;
  }

  async clear(): Promise<boolean> {
    this.mirror.clear();
    try {
      const allKeys = await AsyncStorage.getAllKeys();
      const mine = allKeys.filter((k) => k.startsWith(this.prefix));
      await Promise.all(mine.map((k) => AsyncStorage.removeItem(k)));
      return true;
    } catch {
      return false;
    }
  }
}
