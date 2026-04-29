import { SyncEngine, type Row } from "@pylonsync/sync";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { callFn, getBaseUrl, getReactStorage, storageKey } from "./index";

// ---------------------------------------------------------------------------
// Query shapes
// ---------------------------------------------------------------------------

/** Operator-based filter matching the server's query_filtered API. */
export type QueryFilter = Record<string, unknown> & {
  $order?: Record<string, "asc" | "desc">;
  $limit?: number;
};

/** Include syntax for nested relations: `{ author: {}, tags: {} }`. */
export type IncludeSpec = Record<string, Record<string, unknown>>;

export interface QueryOptions {
  /** Filter by fields and operators (server-side). */
  where?: QueryFilter;
  /** Expand relations inline (server-side graph query). */
  include?: IncludeSpec;
  /** Limit number of rows. */
  limit?: number;
  /** Order by field(s). */
  orderBy?: Record<string, "asc" | "desc">;
}

export interface UseQueryReturn<T> {
  data: T[];
  loading: boolean;
  error: Error | null;
  /** Re-fetch from the server. Rarely needed — data is live. */
  refetch: () => void;
}

export interface UseQueryOneReturn<T> {
  data: T | null;
  loading: boolean;
  error: Error | null;
  refetch: () => void;
}

// ---------------------------------------------------------------------------
// useQuery — high-level hook returning {data, loading, error}
// ---------------------------------------------------------------------------

/**
 * Live query hook. Returns rows for an entity with loading/error state.
 *
 * Automatically re-renders when underlying data changes via the sync engine.
 *
 * ```tsx
 * const { data: todos, loading, error } = useQuery<Todo>(sync, "Todo");
 * ```
 *
 * With filters and ordering:
 *
 * ```tsx
 * const { data } = useQuery<Todo>(sync, "Todo", {
 *   where: { done: false, priority: { $gte: 3 } },
 *   orderBy: { createdAt: "desc" },
 *   limit: 20,
 * });
 * ```
 *
 * Filter/order/limit are applied client-side against the sync store;
 * the sync engine pulls the full entity in the background.
 */
export function useQuery<T = Row>(
  sync: SyncEngine,
  entity: string,
  options?: QueryOptions
): UseQueryReturn<T> {
  const loading = useRef<boolean>(sync.store.list(entity).length === 0);
  const error = useRef<Error | null>(null);
  const optionsKey = JSON.stringify(options || {});

  // Subscribe function stable across the lifetime of this entity/options combo.
  const subscribe = useMemo(
    () => (onChange: () => void) => {
      return sync.store.subscribe((changedEntity?: string) => {
        if (!changedEntity || changedEntity === entity) {
          onChange();
        }
      });
    },
    [sync, entity]
  );

  // Cache the filtered snapshot so getSnapshot returns a stable reference
  // while the underlying data is unchanged.
  const snapshotCache = useRef<{ rows: T[]; sig: string }>({
    rows: [],
    sig: "__init__",
  });

  const getSnapshot = useCallback((): T[] => {
    const rows = sync.store.list(entity) as Row[];
    const filtered = applyClientFilter(rows, options);
    const sig = optionsKey + ":" + JSON.stringify(filtered);
    if (sig !== snapshotCache.current.sig) {
      snapshotCache.current = { rows: filtered as T[], sig };
    }
    if (rows.length > 0 && loading.current) loading.current = false;
    return snapshotCache.current.rows;
  }, [sync, entity, optionsKey, options]);

  const getServerSnapshot = useCallback((): T[] => [] as T[], []);

  const data = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  const refetch = useCallback(() => {
    loading.current = true;
    error.current = null;
    sync.pull().catch((e: unknown) => {
      error.current = e instanceof Error ? e : new Error(String(e));
    });
  }, [sync]);

  return {
    data,
    loading: loading.current,
    error: error.current,
    refetch,
  };
}

/**
 * Live single-row query by ID. Returns the row or null, with loading/error state.
 *
 * ```tsx
 * const { data: todo, loading } = useQueryOne<Todo>(sync, "Todo", todoId);
 * ```
 */
export function useQueryOne<T = Row>(
  sync: SyncEngine,
  entity: string,
  id: string
): UseQueryOneReturn<T> {
  const loading = useRef<boolean>(sync.store.get(entity, id) === null);
  const error = useRef<Error | null>(null);

  const subscribe = useMemo(
    () => (onChange: () => void) => {
      return sync.store.subscribe((changedEntity?: string) => {
        if (!changedEntity || changedEntity === entity) {
          onChange();
        }
      });
    },
    [sync, entity]
  );

  const snapshotCache = useRef<{ row: T | null; sig: string }>({
    row: null,
    sig: "__init__",
  });

  const getSnapshot = useCallback((): T | null => {
    const row = sync.store.get(entity, id) as Row | null;
    const sig = JSON.stringify(row);
    if (sig !== snapshotCache.current.sig) {
      snapshotCache.current = { row: (row as T) ?? null, sig };
    }
    if (row !== null && loading.current) loading.current = false;
    return snapshotCache.current.row;
  }, [sync, entity, id]);

  const getServerSnapshot = useCallback((): T | null => null, []);

  const data = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  const refetch = useCallback(() => {
    loading.current = true;
    error.current = null;
    sync.pull().catch((e: unknown) => {
      error.current = e instanceof Error ? e : new Error(String(e));
    });
  }, [sync]);

  return { data, loading: loading.current, error: error.current, refetch };
}

// ---------------------------------------------------------------------------
// Client-side filter application (matches the server's operator set)
// ---------------------------------------------------------------------------

function applyClientFilter(rows: Row[], options?: QueryOptions): Row[] {
  if (!options) return rows;

  let out = rows.slice();
  if (options.where) {
    out = out.filter((row) => matchesWhere(row, options.where!));
  }
  if (options.orderBy) {
    for (const [field, dir] of Object.entries(options.orderBy)) {
      out.sort((a, b) => compare(a[field], b[field], dir));
    }
  }
  if (typeof options.limit === "number") {
    out = out.slice(0, options.limit);
  }
  return out;
}

function matchesWhere(row: Row, where: QueryFilter): boolean {
  for (const [key, val] of Object.entries(where)) {
    if (key === "$order" || key === "$limit") continue;
    const rowVal = row[key];

    if (val !== null && typeof val === "object" && !Array.isArray(val)) {
      // Operator object.
      for (const [op, opVal] of Object.entries(val as Record<string, unknown>)) {
        switch (op) {
          case "$not":
            if (rowVal === opVal) return false;
            break;
          case "$gt":
            if (!(typeof rowVal === "number" && typeof opVal === "number" && rowVal > opVal))
              return false;
            break;
          case "$gte":
            if (!(typeof rowVal === "number" && typeof opVal === "number" && rowVal >= opVal))
              return false;
            break;
          case "$lt":
            if (!(typeof rowVal === "number" && typeof opVal === "number" && rowVal < opVal))
              return false;
            break;
          case "$lte":
            if (!(typeof rowVal === "number" && typeof opVal === "number" && rowVal <= opVal))
              return false;
            break;
          case "$like":
            if (
              !(typeof rowVal === "string" && typeof opVal === "string" && rowVal.includes(opVal))
            )
              return false;
            break;
          case "$in":
            if (!Array.isArray(opVal) || !(opVal as unknown[]).includes(rowVal)) return false;
            break;
        }
      }
    } else {
      if (rowVal !== val) return false;
    }
  }
  return true;
}

function compare(a: unknown, b: unknown, dir: "asc" | "desc"): number {
  const mult = dir === "desc" ? -1 : 1;
  if (a === b) return 0;
  if (a === undefined || a === null) return mult;
  if (b === undefined || b === null) return -mult;
  if (typeof a === "number" && typeof b === "number") return (a - b) * mult;
  return String(a).localeCompare(String(b)) * mult;
}

// ---------------------------------------------------------------------------
// useMutation — call a server-side TypeScript function
// ---------------------------------------------------------------------------

export interface UseMutationReturn<TArgs, TResult> {
  mutate: (args: TArgs) => Promise<TResult>;
  mutateAsync: (args: TArgs) => Promise<TResult>;
  loading: boolean;
  data: TResult | null;
  error: Error | null;
  reset: () => void;
}

/**
 * Hook for calling a server-side mutation/action function.
 *
 * ```tsx
 * const placeBid = useMutation<{lotId: string; amount: number}, {accepted: boolean}>(
 *   "placeBid"
 * );
 *
 * const onClick = async () => {
 *   const result = await placeBid.mutate({ lotId: "lot_1", amount: 150 });
 *   if (result.accepted) alert("Bid placed!");
 * };
 * ```
 */
export function useMutation<TArgs = Record<string, unknown>, TResult = unknown>(
  fnName: string,
  options: { token?: string } = {}
): UseMutationReturn<TArgs, TResult> {
  const [loading, setLoading] = useState(false);
  const [data, setData] = useState<TResult | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const tokenRef = useRef(options.token);
  tokenRef.current = options.token;

  // mounted guard: a mutate() kicked off right before unmount used to
  // resolve after cleanup and call set{Data,Error,Loading} on a dead
  // component, producing React warnings in dev and silently wasted work
  // in prod. Skip state updates when the component is gone.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const mutate = useCallback(
    async (args: TArgs): Promise<TResult> => {
      if (mounted.current) setLoading(true);
      if (mounted.current) setError(null);
      try {
        const result = await callFn<TResult>(
          fnName,
          args as Record<string, unknown>,
          { token: tokenRef.current }
        );
        if (mounted.current) setData(result);
        return result;
      } catch (e) {
        const err = e instanceof Error ? e : new Error(String(e));
        if (mounted.current) setError(err);
        throw err;
      } finally {
        if (mounted.current) setLoading(false);
      }
    },
    [fnName]
  );

  const reset = useCallback(() => {
    if (!mounted.current) return;
    setData(null);
    setError(null);
  }, []);

  return {
    mutate,
    mutateAsync: mutate,
    loading,
    data,
    error,
    reset,
  };
}

// ---------------------------------------------------------------------------
// useInfiniteQuery — paginated live query with loadMore()
// ---------------------------------------------------------------------------

export interface UseInfiniteQueryReturn<T> {
  data: T[];
  loading: boolean;
  hasMore: boolean;
  loadMore: () => void;
  error: Error | null;
}

/**
 * Paginated query hook that accumulates pages as you `loadMore()`.
 *
 * ```tsx
 * const { data, hasMore, loadMore, loading } = useInfiniteQuery<Todo>(
 *   sync, "Todo", { pageSize: 20 }
 * );
 * ```
 */
export function useInfiniteQuery<T = Row>(
  sync: SyncEngine,
  entity: string,
  options: { pageSize?: number } = {}
): UseInfiniteQueryReturn<T> {
  const pageSize = options.pageSize ?? 20;
  const [data, setData] = useState<T[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const offsetRef = useRef<number>(0);

  // Mounted guard + in-flight ref. Two related issues:
  //   1. setState after unmount — same problem as useMutation.
  //   2. Concurrent loadMore() calls read stale `loading`/`hasMore` from
  //      the render closure (the guard at the top of loadMore reads the
  //      last-rendered value, not the live one). Use a ref for the
  //      in-flight bit so back-to-back loadMore() can't queue duplicate
  //      `loadPage` calls.
  const mounted = useRef(true);
  const inFlight = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const loadMore = useCallback(() => {
    if (inFlight.current || !hasMore) return;
    inFlight.current = true;
    if (mounted.current) setLoading(true);
    if (mounted.current) setError(null);
    sync
      .loadPage(entity, { offset: offsetRef.current, limit: pageSize })
      .then((result) => {
        offsetRef.current += result.data.length;
        if (mounted.current) {
          setHasMore(result.hasMore);
          setData((prev) => [...prev, ...(result.data as T[])]);
        }
      })
      .catch((e: unknown) => {
        if (mounted.current) {
          setError(e instanceof Error ? e : new Error(String(e)));
        }
      })
      .finally(() => {
        inFlight.current = false;
        if (mounted.current) setLoading(false);
      });
  }, [sync, entity, pageSize, hasMore]);

  // Load first page on mount.
  useEffect(() => {
    if (data.length === 0 && hasMore && !loading) {
      loadMore();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { data, loading, hasMore, loadMore, error };
}

// ---------------------------------------------------------------------------
// usePaginatedQuery — Convex-compatible status enum API
// ---------------------------------------------------------------------------

export type PaginatedQueryStatus =
  | "LoadingFirstPage"
  | "CanLoadMore"
  | "LoadingMore"
  | "Exhausted";

export interface UsePaginatedQueryReturn<T> {
  /** Rows loaded so far, across all pages. */
  results: T[];
  /** State-machine value — render based on this rather than booleans. */
  status: PaginatedQueryStatus;
  /** Fetch the next page. Idempotent: no-op while loading or exhausted. */
  loadMore: (numItems?: number) => void;
  /** The most recent error, if any. Resets on the next successful load. */
  error: Error | null;
}

/**
 * Cursor-paginated live query. Pairs with `ctx.db.paginate()` server-side
 * and the `GET /api/entities/:entity/cursor` endpoint.
 *
 * ```tsx
 * const { results, status, loadMore } = usePaginatedQuery<Order>(
 *   sync,
 *   "Order",
 *   { initialNumItems: 20 }
 * );
 *
 * return (
 *   <>
 *     {results.map(o => <Row key={o.id} order={o} />)}
 *     {status === "CanLoadMore" && <button onClick={() => loadMore()}>More</button>}
 *     {status === "LoadingMore" && <Spinner />}
 *     {status === "Exhausted" && <footer>end</footer>}
 *   </>
 * );
 * ```
 *
 * Same engine as `useInfiniteQuery`; different surface. Prefer this one in
 * new code — the `status` enum makes exhaustive rendering easier to get
 * right than `hasMore/loading` booleans.
 */
export function usePaginatedQuery<T = Row>(
  sync: SyncEngine,
  entity: string,
  options: { initialNumItems?: number } = {},
): UsePaginatedQueryReturn<T> {
  const initial = options.initialNumItems ?? 20;
  const inner = useInfiniteQuery<T>(sync, entity, { pageSize: initial });

  let status: PaginatedQueryStatus;
  if (inner.loading && inner.data.length === 0) {
    status = "LoadingFirstPage";
  } else if (inner.loading) {
    status = "LoadingMore";
  } else if (!inner.hasMore) {
    status = "Exhausted";
  } else {
    status = "CanLoadMore";
  }

  return {
    results: inner.data,
    status,
    loadMore: () => inner.loadMore(),
    error: inner.error,
  };
}

// ---------------------------------------------------------------------------
// Raw hooks (backward-compat) — exposes useSyncExternalStore triples
// ---------------------------------------------------------------------------

/**
 * Low-level hook returning `{subscribe, getSnapshot, getServerSnapshot}` for
 * `useSyncExternalStore`. Prefer [`useQuery`] above for most cases; use this
 * when you need precise control over subscription timing.
 */
export function useQueryRaw(sync: SyncEngine, entity: string) {
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

export function useQueryOneRaw(sync: SyncEngine, entity: string, id: string) {
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

// ---------------------------------------------------------------------------
// Legacy CRUD mutations (sync-engine-backed) — renamed to avoid collision
// ---------------------------------------------------------------------------

/**
 * Entity-level CRUD helpers backed by the sync engine (optimistic updates).
 * Separate from [`useMutation`] which calls server-side TypeScript functions.
 */
export function useEntityMutation(sync: SyncEngine, entity: string) {
  return {
    insert: (data: Row) => sync.insert(entity, data),
    update: (id: string, data: Partial<Row>) => sync.update(entity, id, data),
    remove: (id: string) => sync.delete(entity, id),
  };
}

export const useLiveList = useQueryRaw;
export const useLiveRow = useQueryOneRaw;

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
  actionFn: (data: Row) => Promise<void>
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

// ---------------------------------------------------------------------------
// useFn — legacy alias for useMutation (kept for back-compat)
// ---------------------------------------------------------------------------

export interface UseFnReturn<TResult> {
  call: (args?: Record<string, unknown>) => Promise<TResult>;
  loading: boolean;
  data: TResult | null;
  error: Error | null;
  reset: () => void;
}

/**
 * Call a server-side function with loading/error/data state.
 * Prefer [`useMutation`] for new code — same functionality, better API.
 */
export function useFn<TResult = unknown>(
  name: string,
  options: { token?: string } = {}
): UseFnReturn<TResult> {
  const m = useMutation<Record<string, unknown>, TResult>(name, options);
  return {
    call: (args: Record<string, unknown> = {}) => m.mutate(args),
    loading: m.loading,
    data: m.data,
    error: m.error,
    reset: m.reset,
  };
}

// ---------------------------------------------------------------------------
// useAggregate — live count/sum/avg/groupBy queries for dashboards
// ---------------------------------------------------------------------------

/**
 * Aggregate spec — server matches this shape in
 * `POST /api/aggregate/:entity`. The server auto-injects an `orgId`
 * clamp into `where` when the caller has a tenant, so a malicious
 * client can't sum across orgs.
 */
export interface AggregateSpec {
  /** "*" for COUNT(*), a column name for COUNT(col). */
  count?: string;
  /** Columns to sum. */
  sum?: string[];
  /** Columns to average. */
  avg?: string[];
  /** Columns to take the minimum of. */
  min?: string[];
  /** Columns to take the maximum of. */
  max?: string[];
  /** Columns to COUNT DISTINCT. */
  countDistinct?: string[];
  /**
   * Group keys. Each entry is either a column name, or a date-bucket
   * spec `{ field, bucket }` where bucket ∈ hour/day/week/month/year.
   */
  groupBy?: (string | { field: string; bucket: "hour" | "day" | "week" | "month" | "year" })[];
  /** Equality filter applied before aggregation. */
  where?: Record<string, unknown>;
}

export interface UseAggregateReturn<Row = Record<string, unknown>> {
  data: Row[] | null;
  loading: boolean;
  error: Error | null;
  /** Re-run the query. Rarely needed — the hook refreshes on sync notify. */
  refresh: () => void;
}

/**
 * Run an aggregate query and keep it fresh as the sync store mutates.
 *
 * The hook re-fetches whenever the given entity changes in the local
 * sync replica — so charts stay live without polling. Subscribes to
 * the entity's sync events; any change triggers a debounced re-fetch.
 *
 * ```tsx
 * const { data } = useAggregate(sync, "Order", {
 *   count: "*",
 *   groupBy: [{ field: "createdAt", bucket: "day" }],
 *   where: { status: "delivered" },
 * });
 * ```
 */
export function useAggregate<Row = Record<string, unknown>>(
  sync: SyncEngine,
  entity: string,
  spec: AggregateSpec,
): UseAggregateReturn<Row> {
  const [data, setData] = useState<Row[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  // Stringify the spec so we only refetch when the semantic query changes,
  // not on every parent render (spec object is usually a literal).
  const specKey = JSON.stringify(spec);

  const run = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const baseUrl = getBaseUrl();
      const token = getReactStorage().get(storageKey("token"));
      const res = await fetch(`${baseUrl}/api/aggregate/${entity}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: specKey,
      });
      const json = (await res.json()) as { rows?: Row[]; error?: { message: string } };
      if (!res.ok) {
        throw new Error(json.error?.message || `HTTP ${res.status}`);
      }
      setData(json.rows ?? []);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entity, specKey]);

  // Initial fetch + refetch on spec change.
  useEffect(() => {
    void run();
  }, [run]);

  // Live refresh: re-run whenever the sync store notifies a change for
  // this entity (or any entity — pessimistic, but debounced). Keeps
  // charts in sync with writes without manual polling.
  useEffect(() => {
    let pending: ReturnType<typeof setTimeout> | null = null;
    const unsub = sync.store.subscribe((changedEntity?: string) => {
      if (changedEntity && changedEntity !== entity) return;
      if (pending) clearTimeout(pending);
      // 150ms debounce — burst writes (bulk import, WS replay) collapse
      // into a single refetch instead of hammering the aggregate endpoint.
      pending = setTimeout(() => {
        void run();
      }, 150);
    });
    return () => {
      if (pending) clearTimeout(pending);
      unsub();
    };
  }, [sync, entity, run]);

  return { data, loading, error, refresh: run };
}

// ---------------------------------------------------------------------------
// useSearch — faceted full-text search with live facet count updates
// ---------------------------------------------------------------------------

export interface SearchSpec {
  /** Free-text match across the entity's declared `text` fields. */
  query?: string;
  /** Equality filters. Keys must be facet fields in the entity's schema. */
  filters?: Record<string, string | number | boolean>;
  /** Facet fields to return counts for. If omitted, all declared facets. */
  facets?: string[];
  /** Sort by `[field, "asc" | "desc"]`. Field must be in `sortable`. */
  sort?: [string, "asc" | "desc"];
  /** Zero-indexed page. Default 0. */
  page?: number;
  /** Results per page. Clamped server-side to 1..=100. Default 20. */
  pageSize?: number;
}

export interface UseSearchReturn<T = Row> {
  /** The current page of hits, already sorted. */
  hits: T[];
  /** `{facet: {value: count}}` for every declared (or requested) facet. */
  facetCounts: Record<string, Record<string, number>>;
  /** Total hit count across all pages. */
  total: number;
  /** Server-reported query latency in ms. */
  tookMs: number;
  loading: boolean;
  error: Error | null;
  refresh: () => Promise<void>;
}

/**
 * Live faceted search hook. Wraps the `POST /api/search/:entity`
 * endpoint, re-runs the query when the sync replica signals a write
 * on the target entity, and returns ranked hits plus live facet
 * counts in one call.
 *
 * ```tsx
 * const { hits, facetCounts, total, loading } = useSearch<Product>(
 *   sync, "Product",
 *   {
 *     query: "red sneakers",
 *     filters: { category: "shoes" },
 *     facets: ["brand", "color"],
 *     sort: ["price", "desc"],
 *     page: 0, pageSize: 20,
 *   },
 * );
 * ```
 *
 * Live-update model matches `useAggregate`: subscribes to the sync
 * store and re-fetches on any change for this entity. Facet counts
 * reflect server-computed bitmap intersections — adding/removing a
 * Product row drops the freshly-recomputed counts back into the UI
 * in under 100ms on typical catalogs.
 */
export function useSearch<T = Row>(
  sync: SyncEngine,
  entity: string,
  spec: SearchSpec,
): UseSearchReturn<T> {
  const [hits, setHits] = useState<T[]>([]);
  const [facetCounts, setFacetCounts] = useState<
    Record<string, Record<string, number>>
  >({});
  const [total, setTotal] = useState(0);
  const [tookMs, setTookMs] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Key the debounce on the semantic query shape so parent re-renders
  // with the same spec literal don't trigger spurious fetches.
  const specKey = JSON.stringify(spec);

  // Monotonic request counter + AbortController — every `run()` grabs
  // a fresh id, aborts the previous in-flight request at the transport,
  // and refuses to apply its results if a newer request kicked off
  // before it resolved. Without this, typing quickly would race: the
  // older slower response would overwrite the newer one and the UI
  // would show stale hits / facet counts.
  const requestIdRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);

  const run = useCallback(async () => {
    requestIdRef.current += 1;
    const myId = requestIdRef.current;
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    setLoading(true);
    setError(null);
    try {
      const baseUrl = getBaseUrl();
      const token = getReactStorage().get(storageKey("token"));
      const body = JSON.stringify({
        query: spec.query ?? "",
        filters: spec.filters ?? {},
        facets: spec.facets ?? [],
        sort: spec.sort,
        page: spec.page ?? 0,
        page_size: spec.pageSize ?? 20,
      });
      const res = await fetch(`${baseUrl}/api/search/${entity}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body,
        signal: controller.signal,
      });
      const json = (await res.json()) as {
        hits?: T[];
        facetCounts?: Record<string, Record<string, number>>;
        total?: number;
        tookMs?: number;
        error?: { message: string };
      };
      if (myId !== requestIdRef.current) return; // stale — newer in flight
      if (!res.ok) {
        throw new Error(json.error?.message ?? `HTTP ${res.status}`);
      }
      setHits(json.hits ?? []);
      setFacetCounts(json.facetCounts ?? {});
      setTotal(json.total ?? 0);
      setTookMs(json.tookMs ?? 0);
    } catch (e) {
      if (myId !== requestIdRef.current) return; // stale — ignore
      if ((e as Error)?.name === "AbortError") return;
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      if (myId === requestIdRef.current) setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entity, specKey]);

  // Initial fetch + re-fetch when the semantic spec changes.
  useEffect(() => {
    void run();
  }, [run]);

  // Live refresh: subscribe to sync events, re-run on any change that
  // touches this entity. 150ms debounce coalesces burst writes (WS
  // replay, bulk import) into one refetch.
  useEffect(() => {
    let pending: ReturnType<typeof setTimeout> | null = null;
    const unsub = sync.store.subscribe((changedEntity?: string) => {
      if (changedEntity && changedEntity !== entity) return;
      if (pending) clearTimeout(pending);
      pending = setTimeout(() => {
        void run();
      }, 150);
    });
    return () => {
      if (pending) clearTimeout(pending);
      unsub();
    };
  }, [sync, entity, run]);

  return { hits, facetCounts, total, tookMs, loading, error, refresh: run };
}
