import {
  SyncEngine,
  createSyncEngine,
  type Row,
  type SyncEngineConfig,
} from "@statecraft/sync";
import { useQuery, useQueryOne, useMutation } from "./hooks";

// ---------------------------------------------------------------------------
// db — one-liner API (like InstantDB's db.useQuery)
//
// Identical to @statecraft/react/db but imports hooks from the local RN module
// to avoid subpath resolution issues with Metro.
// ---------------------------------------------------------------------------

let _sync: SyncEngine | null = null;
let _started = false;

/**
 * Initialize the statecraft client. Call once at app startup.
 *
 * ```ts
 * import { init } from "@statecraft/react-native";
 * init({ baseUrl: "http://localhost:4321" });
 * ```
 */
export function init(
  config?: Partial<SyncEngineConfig> & { baseUrl?: string },
) {
  _sync = createSyncEngine(
    config?.baseUrl ?? "http://localhost:4321",
    config,
  );
  _started = false;
}

function getSync(): SyncEngine {
  if (!_sync) {
    _sync = createSyncEngine("http://localhost:4321");
  }
  if (!_started) {
    _started = true;
    _sync.start();
  }
  return _sync;
}

/**
 * One-line live query. Returns { subscribe, getSnapshot, getServerSnapshot }
 * for use with useSyncExternalStore.
 *
 * ```tsx
 * import { useSyncExternalStore } from "react";
 * import { db } from "@statecraft/react-native";
 *
 * function TodoList() {
 *   const todos = useSyncExternalStore(...db.useQuery("Todo"));
 *   return <FlatList data={todos} renderItem={...} />;
 * }
 * ```
 */
export const db = {
  /** Live query for all rows of an entity. */
  useQuery(entity: string) {
    const sync = getSync();
    const q = useQuery(sync, entity);
    return [q.subscribe, q.getSnapshot, q.getServerSnapshot] as const;
  },

  /** Live query for a single row by ID. */
  useQueryOne(entity: string, id: string) {
    const sync = getSync();
    const q = useQueryOne(sync, entity, id);
    return [q.subscribe, q.getSnapshot, q.getServerSnapshot] as const;
  },

  /** Get mutation helpers for an entity. */
  useMutation(entity: string) {
    return useMutation(getSync(), entity);
  },

  /** Get the sync engine instance. */
  get sync() {
    return getSync();
  },

  /** Insert a row. */
  insert(entity: string, data: Row) {
    return getSync().insert(entity, data);
  },

  /** Update a row. */
  update(entity: string, id: string, data: Partial<Row>) {
    return getSync().update(entity, id, data);
  },

  /** Delete a row. */
  delete(entity: string, id: string) {
    return getSync().delete(entity, id);
  },

  /** Set presence data. */
  setPresence(data: Record<string, unknown>) {
    (
      getSync() as unknown as {
        setPresence: (d: Record<string, unknown>) => void;
      }
    ).setPresence(data);
  },

  /** Publish to a topic. */
  publishTopic(topic: string, data: unknown) {
    (
      getSync() as unknown as {
        publishTopic: (t: string, d: unknown) => void;
      }
    ).publishTopic(topic, data);
  },
};
