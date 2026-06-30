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
  getReactStorage,
  storageKey,
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
  // Match createSyncEngine's resolution: when init({ appName }) omits
  // baseUrl, fall back to window.location.origin so callFn / fetchList /
  // session helpers go through the same dev-server proxy as the sync
  // engine. Otherwise callFn hits the hard-coded http://localhost:4321
  // default and trips CORS preflights in any Vite/Next setup where the
  // page origin doesn't equal the API origin.
  const resolvedBaseUrl =
    config?.baseUrl && config.baseUrl.length > 0
      ? config.baseUrl
      : typeof window !== "undefined" && window.location?.origin
        ? window.location.origin
        : undefined;
  configureClient({
    baseUrl: resolvedBaseUrl,
    appName: config?.appName,
  });
}

/**
 * Switch the active session token and re-sync the local replica to the new
 * identity. Call this on EVERY auth transition — login, signup, AND logout
 * (pass `null`).
 *
 * It writes the token where both the request helpers and the sync engine read
 * it, then wipes the local replica — dropping the previous identity's pending
 * offline writes so they can't replay as the new user — and re-snapshots under
 * the new identity.
 *
 * Why this exists: changing the stored token alone is NOT enough. The engine is
 * already running with the previous identity's replica + cursor; without an
 * explicit reset it keeps serving that (or, after a re-init, an empty replica on
 * a stale cursor) — so the rows the new user should see never load and freshly
 * written rows appear to "vanish" on the next render/refresh. This is the
 * supported, can't-get-it-wrong way to flip identity; hand-rolling token writes
 * without it is the #1 cause of "my data disappears after login" in Pylon apps.
 */
export async function setSessionToken(token: string | null): Promise<void> {
  const key = storageKey("token");
  const store = getReactStorage();
  if (token) store.set(key, token);
  else store.remove(key);
  // resetReplica wipes rows + cursor (wipeMutations drops the outgoing
  // identity's queued writes) so the previous identity's data is gone
  // immediately. The re-snapshot pull runs in the BACKGROUND (not awaited) — for
  // a large replica it can take a moment, and blocking login on a full snapshot
  // is bad UX; live `useQuery`/`useSearch` hooks show loading and populate as the
  // snapshot lands. Awaiting resetReplica (fast) is enough for callers.
  const engine = getSync();
  await engine.resetReplica({ wipeMutations: true });
  void engine.pull();
}

/** Module-internal accessor for the global sync engine. Exported so
 *  hooks living outside this file (e.g. `useRoom`) can share the same
 *  engine instance and benefit from the same lazy-start / lazy-init
 *  semantics — without re-implementing the resolution rules. */
export function getSync(): SyncEngine {
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
   * Routes through `SyncEngine.fn` (not the free `callFn`) so the response's
   * `X-Pylon-Change-Seq` header triggers a fallback pull when the WS
   * broadcast for the same event hasn't landed yet — closes the gap where
   * a mutation succeeds but the cached query doesn't observe the new row.
   *
   * ```ts
   * const result = await db.fn("placeBid", { lotId: "x", amount: 150 });
   * ```
   */
  fn<T = unknown>(name: string, args?: Record<string, unknown>): Promise<T> {
    return getSync().fn<T>(name, args);
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
