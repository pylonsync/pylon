import { SyncEngine, createSyncEngine, type Row, type SyncEngineConfig } from "@pylonsync/sync";
import {
  useQuery as useQueryHook,
  useQueryOne as useQueryOneHook,
  useMutation as useMutationHook,
  useInfiniteQuery as useInfiniteQueryHook,
  useAggregate as useAggregateHook,
  useSearch as useSearchHook,
  useEntityMutation,
  type QueryOptions,
  type UseQueryReturn,
  type UseQueryOneReturn,
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
 */
export function init(config?: Partial<SyncEngineConfig> & { baseUrl?: string }) {
  _sync = createSyncEngine(config?.baseUrl ?? "http://localhost:4321", config);
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
    _sync = createSyncEngine("http://localhost:4321");
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
   * Server-side function call with mutation state (loading, data, error).
   *
   * ```tsx
   * const placeBid = db.useMutation<{lotId: string}, {accepted: boolean}>("placeBid");
   * await placeBid.mutate({ lotId: "x", amount: 150 });
   * ```
   */
  useMutation<TArgs = Record<string, unknown>, TResult = unknown>(
    fnName: string
  ): UseMutationReturn<TArgs, TResult> {
    return useMutationHook<TArgs, TResult>(fnName);
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
