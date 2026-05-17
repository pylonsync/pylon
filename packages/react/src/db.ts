"use client";

import { SyncEngine, createSyncEngine, type Row, type SyncEngineConfig } from "@pylonsync/sync";
import {
  useQuery as useQueryHook,
  useQueryOne as useQueryOneHook,
  useReactiveQuery as useReactiveQueryHook,
  useMutation as useMutationHook,
  useInfiniteQuery as useInfiniteQueryHook,
  useAggregate as useAggregateHook,
  useSearch as useSearchHook,
  useEntityMutation,
  type QueryOptions,
  type UseQueryReturn,
  type UseQueryOneReturn,
  type UseReactiveQueryReturn,
  type UseMutationReturn,
  type UseInfiniteQueryReturn,
  type AggregateSpec,
  type UseAggregateReturn,
  type SearchSpec,
  type UseSearchReturn,
} from "./hooks";
import {
  callFn,
  configureClient,
  streamFn,
  uploadFile,
  uploadFileMultipart,
  type UploadedFile,
} from "./index";

// ---------------------------------------------------------------------------
// db — one-liner API
// ---------------------------------------------------------------------------

let _sync: SyncEngine | null = null;
let _started = false;

/**
 * Initialize the pylon client. Call once at app startup.
 *
 * ```ts
 * import { init } from "@pylonsync/react";
 * init({ baseUrl: "http://localhost:4321" });
 * ```
 *
 * Omitting `baseUrl` in a browser context falls back to
 * `window.location.origin` — the right answer for same-origin
 * deployments (Next.js + Vercel rewrites, embedded SPA). Passing an
 * explicit `baseUrl` always wins. We deliberately do NOT default to
 * `http://localhost:4321` in browsers — that footgun caused production
 * dashboards to fire requests at the engineer's dev port.
 */
export function init(config?: Partial<SyncEngineConfig> & { baseUrl?: string }) {
  _sync = createSyncEngine(config?.baseUrl, config);
  _started = false;
  // Keep the React-side helpers in sync — a single init() should fully
  // namespace this app's storage without a separate configureClient call.
  configureClient({
    baseUrl: config?.baseUrl,
    appName: config?.appName,
  });
}

function getSync(): SyncEngine {
  if (!_sync) {
    // Lazy fallback for callers that never invoked init(). Same
    // resolution rules as init: browser → window.location.origin,
    // SSR → localhost:4321 (the pylon dev default). The browser case
    // is critical: without it a useQuery hook that fires before the
    // app's SyncProvider effect lands would leak `localhost:4321`
    // requests in production.
    _sync = createSyncEngine();
  }
  if (!_started) {
    _started = true;
    _sync.start();
  }
  return _sync;
}

/**
 * Live query with loading/error state.
 *
 * ```tsx
 * const { data, loading, error } = db.useQuery<Todo>("Todo", {
 *   where: { done: false },
 *   orderBy: { createdAt: "desc" },
 * });
 * ```
 */
export const db = {
  /** Live query for entity rows with loading/error state. */
  useQuery<T = Row>(entity: string, options?: QueryOptions): UseQueryReturn<T> {
    return useQueryHook<T>(getSync(), entity, options);
  },

  /** Live query for a single row by ID. */
  useQueryOne<T = Row>(entity: string, id: string): UseQueryOneReturn<T> {
    return useQueryOneHook<T>(getSync(), entity, id);
  },

  /**
   * Reactive query — Convex-style auto-rerunning server handler.
   *
   * The server runs your `query()` handler with dependency tracking
   * (every `ctx.db.*` read is recorded), registers the subscription,
   * and pushes the initial result. Any future mutation touching the
   * dep set triggers a re-run + push.
   *
   * ```tsx
   * const { data: feed, loading } = db.useReactiveQuery<FeedItem[]>(
   *   "getFeed",
   *   { userId: currentUser.id },
   * );
   * ```
   *
   * Authoring side: define the handler with `query()` from
   * `@pylonsync/functions`. Any handler is eligible — no opt-in flag.
   */
  useReactiveQuery<T = unknown>(
    fnName: string,
    args?: unknown,
  ): UseReactiveQueryReturn<T> {
    return useReactiveQueryHook<T>(getSync(), fnName, args);
  },

  /**
   * Server-side function call with mutation state (loading, data, error).
   *
   * ```tsx
   * const placeBid = db.useMutation<{lotId: string}, {accepted: boolean}>("placeBid");
   * await placeBid.mutate({ lotId: "x", amount: 150 });
   * ```
   *
   * For optimistic UI, pass an `optimistic` builder — the framework
   * paints the row into the local store immediately, threads a
   * matching id through to the server function, and reconciles the
   * canonical broadcast as an in-place merge. See
   * docs/concepts/optimistic-updates for the full pattern.
   *
   * ```tsx
   * const send = db.useMutation<{channelId: string; body: string}, {messageId: string}>(
   *   "sendMessage",
   *   {
   *     optimistic: (args, ctx) => ({
   *       entity: "Message",
   *       data: { id: ctx.id, ...args, authorId: me.id, createdAt: ctx.now },
   *     }),
   *   }
   * );
   * ```
   */
  useMutation<TArgs = Record<string, unknown>, TResult = unknown>(
    fnName: string,
    options: { optimistic?: import("./hooks").OptimisticBuilder<TArgs> } = {}
  ): UseMutationReturn<TArgs, TResult> {
    // db.useMutation is the "I'm using the global sync engine" path —
    // surface getSync() to the underlying hook so the optimistic
    // ghost gets painted into the right store. Apps reaching for the
    // raw `useMutation(fnName, { optimistic, sync })` for a non-global
    // engine still work; this is just the ergonomic default.
    return useMutationHook<TArgs, TResult>(fnName, {
      optimistic: options.optimistic,
      sync: getSync(),
    });
  },

  /** Paginated live query with loadMore(). */
  useInfiniteQuery<T = Row>(
    entity: string,
    options: { pageSize?: number } = {}
  ): UseInfiniteQueryReturn<T> {
    return useInfiniteQueryHook<T>(getSync(), entity, options);
  },

  /**
   * Live aggregate query (count / sum / avg / groupBy). Automatically
   * re-runs when the entity's rows change in the sync replica — dashboard
   * charts stay up to date without polling.
   */
  useAggregate<Row = Record<string, unknown>>(
    entity: string,
    spec: AggregateSpec
  ): UseAggregateReturn<Row> {
    return useAggregateHook<Row>(getSync(), entity, spec);
  },

  /**
   * Live faceted full-text search. Returns ranked hits + per-facet
   * counts + total; re-runs when the entity's rows change so facet
   * counts and result lists stay in lockstep with writes.
   *
   * ```tsx
   * const { hits, facetCounts, total } = db.useSearch<Product>("Product", {
   *   query: "red sneakers",
   *   filters: { category: "shoes" },
   *   facets: ["brand", "color"],
   *   sort: ["price", "desc"],
   * });
   * ```
   */
  useSearch<T = Row>(entity: string, spec: SearchSpec): UseSearchReturn<T> {
    return useSearchHook<T>(getSync(), entity, spec);
  },

  /** Entity-level optimistic CRUD (not server-side functions). */
  useEntity(entity: string) {
    return useEntityMutation(getSync(), entity);
  },

  /** Get the sync engine instance. */
  get sync() {
    return getSync();
  },

  /** Insert a row (optimistic). */
  insert(entity: string, data: Row) {
    return getSync().insert(entity, data);
  },

  /** Update a row (optimistic). */
  update(entity: string, id: string, data: Partial<Row>) {
    return getSync().update(entity, id, data);
  },

  /** Delete a row (optimistic). */
  delete(entity: string, id: string) {
    return getSync().delete(entity, id);
  },

  /** Set presence data. */
  setPresence(data: Record<string, unknown>) {
    (getSync() as unknown as { setPresence: (d: Record<string, unknown>) => void }).setPresence(
      data
    );
  },

  /** Publish to a topic. */
  publishTopic(topic: string, data: unknown) {
    (getSync() as unknown as { publishTopic: (t: string, d: unknown) => void }).publishTopic(
      topic,
      data
    );
  },

  /**
   * Call a server-side function (query, mutation, or action).
   *
   * ```ts
   * const result = await db.fn("placeBid", { lotId: "x", amount: 150 });
   * ```
   */
  fn<T = unknown>(name: string, args?: Record<string, unknown>): Promise<T> {
    return callFn<T>(name, args);
  },

  /**
   * Stream output from a server-side function as SSE chunks.
   *
   * ```ts
   * for await (const chunk of db.streamFn("chat", { message: "hi" })) {
   *   console.log(chunk);
   * }
   * ```
   */
  streamFn(name: string, args?: Record<string, unknown>) {
    return streamFn(name, args);
  },

  /** Upload a file to /api/files/upload. */
  uploadFile(
    input: File | Blob | ArrayBuffer | Uint8Array,
    options?: { filename?: string; contentType?: string }
  ): Promise<UploadedFile> {
    return uploadFile(input, options);
  },

  /** Upload via multipart/form-data with extra fields. */
  uploadFileMultipart(
    file: File | Blob,
    fields?: Record<string, string>
  ): Promise<UploadedFile> {
    return uploadFileMultipart(file, fields);
  },
};
