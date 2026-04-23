import { SyncEngine, type Row } from "@pylonsync/sync";

// ---------------------------------------------------------------------------
// React Native hooks for pylon
//
// These are identical to the @pylonsync/react hooks. They use only React and
// the SyncEngine (which relies on fetch), both available in React Native.
// We duplicate rather than import via subpath to avoid Metro resolution
// issues with packages that don't declare explicit `exports`.
// ---------------------------------------------------------------------------

/**
 * Live query hook — returns subscribe/getSnapshot for useSyncExternalStore.
 * Automatically updates when the entity data changes.
 *
 * Usage:
 * ```tsx
 * import { useSyncExternalStore } from "react";
 * const { subscribe, getSnapshot } = useQuery(sync, "Todo");
 * const todos = useSyncExternalStore(subscribe, getSnapshot);
 * ```
 */
export function useQuery(sync: SyncEngine, entity: string) {
  let cache: Row[] = sync.store.list(entity);
  let cacheKey = JSON.stringify(cache);

  const subscribe = (callback: () => void) => {
    return sync.store.subscribe(() => {
      const next = sync.store.list(entity);
      const nextKey = JSON.stringify(next);
      if (nextKey !== cacheKey) {
        cache = next;
        cacheKey = nextKey;
        callback();
      }
    });
  };

  const getSnapshot = () => cache;
  const getServerSnapshot = () => [] as Row[];

  return { subscribe, getSnapshot, getServerSnapshot };
}

/**
 * Live single-row query — subscribe to a specific row by ID.
 */
export function useQueryOne(sync: SyncEngine, entity: string, id: string) {
  let cache: Row | null = sync.store.get(entity, id);
  let cacheKey = JSON.stringify(cache);

  const subscribe = (callback: () => void) => {
    return sync.store.subscribe(() => {
      const next = sync.store.get(entity, id);
      const nextKey = JSON.stringify(next);
      if (nextKey !== cacheKey) {
        cache = next;
        cacheKey = nextKey;
        callback();
      }
    });
  };

  const getSnapshot = () => cache;
  const getServerSnapshot = () => null as Row | null;

  return { subscribe, getSnapshot, getServerSnapshot };
}

/**
 * Mutation helpers — returns typed functions for insert/update/delete.
 * All mutations go through the sync engine with optimistic updates.
 */
export function useMutation(sync: SyncEngine, entity: string) {
  return {
    insert: (data: Row) => sync.insert(entity, data),
    update: (id: string, data: Partial<Row>) => sync.update(entity, id, data),
    remove: (id: string) => sync.delete(entity, id),
  };
}

// Legacy exports for backward compatibility
export const useLiveList = useQuery;
export const useLiveRow = useQueryOne;

export function useInsert(sync: SyncEngine, entity: string) {
  return (data: Row) => sync.insert(entity, data);
}

export function useUpdate(sync: SyncEngine, entity: string) {
  return (id: string, data: Partial<Row>) => sync.update(entity, id, data);
}

export function useDelete(sync: SyncEngine, entity: string) {
  return (id: string) => sync.delete(entity, id);
}

export function useAction(
  sync: SyncEngine,
  entity: string,
  actionFn: (data: Row) => Promise<void>,
) {
  return async (data: Row) => {
    sync.store.optimisticInsert(entity, data);
    try {
      await actionFn(data);
    } catch {
      // Revert on failure — next pull will correct.
    }
  };
}
