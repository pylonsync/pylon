"use client";

import { SyncEngine, generateId, pylonFetch, type Row } from "@pylonsync/sync";
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
// A single shared empty array reused as every useQuery's SSR snapshot. A fresh
// `[]` per call makes React's useSyncExternalStore warn "The result of
// getServerSnapshot should be cached to avoid an infinite loop", so hand it one
// stable reference.
const EMPTY_SNAPSHOT: readonly unknown[] = [];

// Server snapshot for the settled signal. On the server there is no engine to
// have settled, and rendering "not settled" is what makes SSR emit the skeleton
// the client then replaces — matching what a cold client sees, so hydration
// doesn't swap the tree. Hoisted for the same reason as EMPTY_SNAPSHOT: a
// stable identity, though a bare boolean would compare fine.
const FALSE_SNAPSHOT = (): boolean => false;

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
  // Both are state, not refs. A ref mutated inside an async callback changes
  // nothing React can see: the old `error` ref was assigned in `pull()`'s catch
  // and never re-rendered, so a failed refetch surfaced no error until some
  // unrelated update happened to re-render the component.
  const [error, setError] = useState<Error | null>(null);
  const [refetching, setRefetching] = useState(false);
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
    return snapshotCache.current.rows;
  }, [sync, entity, optionsKey, options]);

  const getServerSnapshot = useCallback((): T[] => EMPTY_SNAPSHOT as T[], []);

  const data = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  // The settled signal gets its OWN subscription, with a boolean snapshot.
  //
  // It used to be a `useRef` flipped inside getSnapshot above, which never
  // reached the render for an entity with no rows. useSyncExternalStore
  // re-renders only when getSnapshot returns a value that isn't Object.is-equal
  // to the last one — and the row snapshot for an empty entity is the same
  // cached array before and after the pull settles. So markInitialSyncSettled()
  // notified, getSnapshot ran, the ref flipped to false, React compared the
  // identical array and bailed out, and the component kept rendering the stale
  // `loading: true` forever. Every empty list — a new account's first dashboard,
  // a fresh entity, an org whose rows all soft-deleted — sat on its skeleton
  // and never reached its empty state.
  //
  // Booleans compare by value, so this snapshot genuinely changes when the
  // engine settles and React re-renders.
  const settled = useSyncExternalStore(
    subscribe,
    useCallback(() => sync.isInitialSyncSettled(), [sync]),
    FALSE_SNAPSHOT,
  );

  // Derived, not latched. Loading means "we don't know yet": no server-confirmed
  // view AND nothing local to show. Gating on the settled signal rather than
  // bare hydration is what stops the empty-state flash — a cold cache keeps
  // this true until the pull confirms, and only then is an empty result a real
  // "no rows". The engine's fallback deadline settles it even offline, so this
  // can't pin. Deriving it also means a replica wipe (org switch, token flip)
  // resets the engine's signal and correctly returns callers to a skeleton
  // instead of flashing "nothing here" over data that is on its way back.
  const loading = refetching || (!settled && data.length === 0);

  // Register interest so the reconcile safety net sweeps this entity
  // even when the local replica has zero rows for it. Without this, a
  // server row in a never-cached entity (created on another surface, a
  // freshly-added entity) stays invisible until a full snapshot / cache
  // clear. The first observe of an empty+hydrated entity also fires a
  // one-shot fetch so it appears on mount. See SyncEngine.observeEntity.
  useEffect(() => {
    sync.observeEntity(entity);
  }, [sync, entity]);

  const refetch = useCallback(() => {
    setRefetching(true);
    setError(null);
    sync
      .pull()
      .catch((e: unknown) => {
        setError(e instanceof Error ? e : new Error(String(e)));
      })
      // Clear on settle, success or failure. The old ref-based version set
      // loading true and left it to getSnapshot to clear, which never ran on a
      // pull that returned no new rows — a refetch over an empty entity pinned
      // loading permanently.
      .finally(() => setRefetching(false));
  }, [sync]);

  return { data, loading, error, refetch };
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
  // Same initial-sync-aware loading semantics as useQuery — see the long
  // comment there for why the settled signal needs its own boolean-valued
  // subscription. This hook had the identical defect and it bit harder: a row
  // that does not exist yields the same `null` snapshot before and after the
  // pull, so React bailed out of the re-render and the caller was pinned in
  // loading instead of ever reaching "not found".
  const [error, setError] = useState<Error | null>(null);
  const [refetching, setRefetching] = useState(false);

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
    return snapshotCache.current.row;
  }, [sync, entity, id]);

  const getServerSnapshot = useCallback((): T | null => null, []);

  const data = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  const settled = useSyncExternalStore(
    subscribe,
    useCallback(() => sync.isInitialSyncSettled(), [sync]),
    FALSE_SNAPSHOT,
  );

  // Loading until the row arrives OR the pull confirms it isn't there. Keeping
  // a cold load in the loading state is what stops a row that DOES exist
  // server-side from flashing "not found" while it is still in flight.
  const loading = refetching || (!settled && data === null);

  // Register interest so reconcile sweeps this entity even with zero
  // local rows. See SyncEngine.observeEntity.
  useEffect(() => {
    sync.observeEntity(entity);
  }, [sync, entity]);

  const refetch = useCallback(() => {
    setRefetching(true);
    setError(null);
    sync
      .pull()
      .catch((e: unknown) => {
        setError(e instanceof Error ? e : new Error(String(e)));
      })
      .finally(() => setRefetching(false));
  }, [sync]);

  return { data, loading, error, refetch };
}

// ---------------------------------------------------------------------------
// useReactiveQuery — Convex-style auto-rerunning server query
// ---------------------------------------------------------------------------

export interface UseReactiveQueryReturn<T> {
  /** Latest server-pushed result. `null` until the initial run lands. */
  data: T | null;
  /** True until the first result lands (or the first error). */
  loading: boolean;
  /** Most recent error from the server-side handler, if any. */
  error: Error | null;
}

/**
 * Subscribe to a server-side `query()` handler with automatic re-run
 * on dependency changes. Mirrors Convex's reactive query model:
 *
 * 1. Mount: client sends `reactive-subscribe` over WS with `fn_name`
 *    + `args`. Server runs the handler under the connection's auth,
 *    records which entities the handler read via `ctx.db.*`, registers
 *    the subscription, and pushes the initial result.
 * 2. On every server-side mutation, the runtime's reactive registry
 *    looks up subs whose dep set overlaps the changed entity, re-runs
 *    them, hashes the result, and pushes only when the hash changed.
 * 3. Unmount: client sends `reactive-unsubscribe`; server tears down
 *    the registration and stops re-running.
 *
 * Auth context for re-runs is captured at subscribe time — the
 * subscriber's identity, not the mutating user's. Policy gates the
 * handler runs at first execution apply on every re-run.
 *
 * ```tsx
 * const { data, loading } = useReactiveQuery<MessageWithAuthor[]>(
 *   sync,
 *   "getMessagesWithAuthors",
 *   { channelId: "c_1" },
 * );
 * ```
 *
 * Args object identity matters: changing the args reference triggers
 * an unsubscribe + resubscribe with a fresh sub_id. Stabilize via
 * `useMemo` if you build args inline on every render.
 */
export function useReactiveQuery<T = unknown>(
  sync: SyncEngine,
  fnName: string,
  args?: unknown,
): UseReactiveQueryReturn<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<Error | null>(null);
  // Stable serialization of args — when the JSON shape changes we
  // re-subscribe. Object identity changes alone don't re-subscribe.
  const argsKey = useMemo(() => JSON.stringify(args ?? null), [args]);

  useEffect(() => {
    const sub_id = generateId();
    setLoading(true);
    setError(null);
    sync.subscribeReactive(sub_id, fnName, args ?? null, (msg) => {
      if (msg.kind === "result") {
        setData(msg.result as T);
        setLoading(false);
        setError(null);
      } else {
        // Reactive error pushes (e.g. handler unavailable) — surface
        // to the consumer and stop spinning.
        setError(
          Object.assign(new Error(msg.message || msg.code), { code: msg.code }),
        );
        setLoading(false);
      }
    });
    return () => {
      sync.unsubscribeReactive(sub_id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sync, fnName, argsKey]);

  return { data, loading, error };
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
 * Builder for the optimistic ghost row painted in the local store
 * before the server function returns. Receives the args passed to
 * `mutate()` plus a `ctx` object the framework fills in for you:
 *
 * - `ctx.id`  — the freshly-minted Pylon-shaped row id (40-char hex)
 *               that the framework also threads into the mutation
 *               args as `_optimisticId`. Use this as the row's `id`
 *               so the optimistic ghost and the canonical broadcast
 *               share the same `row_id` and the WS update is an
 *               in-place merge instead of a delete-then-replace flash.
 * - `ctx.now` — `new Date().toISOString()` evaluated once, so the
 *               optimistic ghost has a `createdAt` that's stable
 *               across the same gesture.
 *
 * Return either a single `{ entity, data }` for the common one-row
 * case or an array for mutations that touch multiple entities (e.g.
 * an "accept invite" that inserts a Membership AND an AuditLog row).
 */
export interface OptimisticContext {
  id: string;
  now: string;
}
export type OptimisticChange = { entity: string; data: Row };
export type OptimisticBuilder<TArgs> = (
  args: TArgs,
  ctx: OptimisticContext,
) => OptimisticChange | OptimisticChange[];

export interface UseMutationOptions<TArgs> {
  token?: string;
  /**
   * Paint a row into the local store immediately, before the server
   * function returns. The row uses `ctx.id` as its `id` and the
   * framework threads that id through the mutation args as
   * `_optimisticId` — your server function should accept it and pass
   * it on to `ctx.db.insert("Entity", { id: args._optimisticId, ... })`
   * (the runtime honors caller-supplied ids for any 40-char hex value).
   *
   * The WS broadcast that follows will carry the same `row_id`, so the
   * canonical row lands as a field-level merge on top of the
   * optimistic ghost — no flash, no temp-row swap, no manual cleanup.
   *
   * On rejection, the optimistic insert is rolled back without leaving
   * a tombstone, so retrying the mutation works.
   */
  optimistic?: OptimisticBuilder<TArgs>;
  /**
   * Active sync engine. Required when `optimistic` is set so the hook
   * can paint the ghost into the right store; ignored otherwise. The
   * `db.useMutation` wrapper supplies this automatically via `getSync`.
   */
  sync?: SyncEngine;
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
 *
 * For optimistic UI, pass an `optimistic` builder. See
 * `OptimisticBuilder` above for the contract.
 */
export function useMutation<TArgs = Record<string, unknown>, TResult = unknown>(
  fnName: string,
  options: UseMutationOptions<TArgs> = {}
): UseMutationReturn<TArgs, TResult> {
  const [loading, setLoading] = useState(false);
  const [data, setData] = useState<TResult | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const tokenRef = useRef(options.token);
  tokenRef.current = options.token;
  // Stash the optimistic builder + sync handle in refs so changes
  // between renders don't blow away in-flight mutations. The mutate
  // closure reads through the ref so every call sees the latest
  // builder without needing to re-bind the callback.
  const optimisticRef = useRef(options.optimistic);
  optimisticRef.current = options.optimistic;
  const syncRef = useRef(options.sync);
  syncRef.current = options.sync;

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

      // Paint the optimistic ghost(s) before kicking off the server
      // call. The id we mint here goes into both the ghost and the
      // mutation args (as `_optimisticId`) so the canonical row that
      // arrives over the WS shares the same `row_id` — the local
      // store treats the broadcast as an in-place merge rather than a
      // new row, and the UI doesn't flash through "ghost → empty →
      // canonical" while we wait for the server response.
      let serverArgs = args as TArgs & { _optimisticId?: string };
      let optimisticIds: Array<{ entity: string; id: string }> = [];
      const sync = syncRef.current;
      const builder = optimisticRef.current;
      if (builder && sync) {
        const id = generateId();
        const now = new Date().toISOString();
        const out = builder(args, { id, now });
        const changes = Array.isArray(out) ? out : [out];
        for (const change of changes) {
          // Force the row's `id` to the framework-minted one even if
          // the builder forgot — the ghost MUST share the row id with
          // the canonical for the merge to land cleanly.
          sync.store.optimisticInsertWithId(change.entity, id, {
            ...change.data,
            id,
          });
          optimisticIds.push({ entity: change.entity, id });
        }
        serverArgs = { ...args, _optimisticId: id };
      }

      try {
        // Route through SyncEngine.fn when one is wired so the response's
        // X-Pylon-Change-Seq triggers a fallback pull if the WS broadcast
        // hasn't landed yet. Falling back to the free callFn for hooks
        // wired without a sync engine (rare — only legacy non-React-init
        // callers) keeps backwards compatibility, but apps using
        // useMutation in the normal `init()` flow now match db.fn's
        // change-seq behavior.
        const result = sync
          ? await sync.fn<TResult>(
              fnName,
              serverArgs as Record<string, unknown>,
            )
          : await callFn<TResult>(
              fnName,
              serverArgs as Record<string, unknown>,
              { token: tokenRef.current },
            );
        if (mounted.current) setData(result);
        return result;
      } catch (e) {
        // Roll back optimistic ghosts without leaving a tombstone — a
        // retry of the same mutation must not be blocked by a dead
        // tombstone seq from this rejected attempt.
        if (sync) {
          for (const { entity, id } of optimisticIds) {
            sync.store.rollbackOptimisticInsert(entity, id);
          }
        }
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
      const json = await pylonFetch<{ rows?: Row[] }>(
        {
          baseUrl: getBaseUrl(),
          getToken: () =>
            (getReactStorage().get(storageKey("token")) ?? undefined) as
              | string
              | undefined,
        },
        `/api/aggregate/${entity}`,
        { method: "POST", body: specKey },
      );
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
      const json = await pylonFetch<{
        hits?: T[];
        facetCounts?: Record<string, Record<string, number>>;
        total?: number;
        tookMs?: number;
      }>(
        {
          baseUrl: getBaseUrl(),
          getToken: () =>
            (getReactStorage().get(storageKey("token")) ?? undefined) as
              | string
              | undefined,
        },
        `/api/search/${entity}`,
        {
          method: "POST",
          json: {
            query: spec.query ?? "",
            filters: spec.filters ?? {},
            facets: spec.facets ?? [],
            sort: spec.sort,
            page: spec.page ?? 0,
            page_size: spec.pageSize ?? 20,
          },
          signal: controller.signal,
        },
      );
      if (myId !== requestIdRef.current) return; // stale — newer in flight
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
