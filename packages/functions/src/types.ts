/**
 * Type definitions for the function system.
 */

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

export interface AuthInfo {
  userId: string | null;
  isAdmin: boolean;
  /** Active tenant id (selected organization) for multi-tenant apps.
   *  Null when the session hasn't selected one. */
  tenantId: string | null;
}

// ---------------------------------------------------------------------------
// Database — read operations
// ---------------------------------------------------------------------------

export interface DbReader {
  /** Get a single row by ID. Returns null if not found. */
  get(entity: string, id: string): Promise<Record<string, unknown> | null>;

  /** List all rows for an entity. */
  list(entity: string): Promise<Record<string, unknown>[]>;

  /** Lookup a row by a field value (e.g., email). */
  lookup(
    entity: string,
    field: string,
    value: string
  ): Promise<Record<string, unknown> | null>;

  /** Query with filters ($gt, $lt, $in, $like, $order, $limit, etc.). */
  query(
    entity: string,
    filter: Record<string, unknown>
  ): Promise<Record<string, unknown>[]>;

  /** Execute a graph query with nested relation includes. */
  queryGraph(
    query: Record<string, unknown>
  ): Promise<Record<string, unknown>>;

  /**
   * Faceted full-text search against an entity that declares a
   * `search:` config. Mirrors the typed-client `client.search()` /
   * the HTTP `/api/search/<entity>` shape.
   *
   * ```ts
   * const result = await ctx.db.search("Product", {
   *   query: "rust async",
   *   filters: { brand: "Atlas" },
   *   facets: ["category"],
   *   page: 0,
   *   pageSize: 20,
   * });
   * ```
   *
   * Returns `{ hits, facetCounts, total, tookMs }`. Throws on
   * entities without a `search:` config (`SEARCH_NOT_CONFIGURED`).
   */
  search(
    entity: string,
    query: Record<string, unknown>
  ): Promise<SearchResult>;

  /**
   * Cursor-paginated list. Pass `cursor` from a previous page's `nextCursor`
   * to continue; pass `null` for the first page.
   *
   * ```ts
   * const { page, nextCursor, isDone } =
   *   await ctx.db.paginate("Order", { cursor: null, numItems: 50 });
   * ```
   *
   * `numItems` is clamped to [1, 1000]; the server honors the clamp.
   */
  paginate(
    entity: string,
    opts: { cursor: string | null; numItems: number }
  ): Promise<PaginationResult>;
}

/** Result shape for [`DbReader.paginate`]. */
export interface PaginationResult<T = Record<string, unknown>> {
  /** Rows in this page. */
  page: T[];
  /** Cursor to pass to the next `paginate` call. `null` when exhausted. */
  nextCursor: string | null;
  /** True when there are no more rows after this page. */
  isDone: boolean;
}

/** Result shape for [`DbReader.search`]. */
export interface SearchResult<T = Record<string, unknown>> {
  /** Ranked (or sorted) hit rows. */
  hits: T[];
  /** `{facet_name: {value: count}}` — counts excluded for the
   *  active filter on the same facet (standard exclusion pattern). */
  facetCounts: Record<string, Record<string, number>>;
  /** Total hit count before pagination. */
  total: number;
  /** Milliseconds spent in the search engine. */
  tookMs: number;
}

// ---------------------------------------------------------------------------
// Database — write operations (extends read)
// ---------------------------------------------------------------------------

export interface DbWriter extends DbReader {
  /** Insert a new row. Returns the generated ID. */
  insert(entity: string, data: Record<string, unknown>): Promise<string>;

  /** Update a row by ID. Returns true if the row existed. */
  update(
    entity: string,
    id: string,
    data: Record<string, unknown>
  ): Promise<boolean>;

  /** Delete a row by ID. Returns true if the row existed. */
  delete(entity: string, id: string): Promise<boolean>;

  /** Link two entities via a relation. */
  link(
    entity: string,
    id: string,
    relation: string,
    targetId: string
  ): Promise<boolean>;

  /** Unlink a relation (set FK to null). */
  unlink(entity: string, id: string, relation: string): Promise<boolean>;
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

export interface Stream {
  /** Write a text chunk to the client (SSE). */
  write(data: string): void;

  /** Write a typed SSE event. */
  writeEvent(event: string, data: string): void;
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

export interface Scheduler {
  /** Schedule a function to run after a delay (milliseconds). */
  runAfter(
    delayMs: number,
    fnName: string,
    args: Record<string, unknown>
  ): Promise<string>;

  /** Schedule a function to run at a specific time (Unix ms). */
  runAt(
    timestamp: number,
    fnName: string,
    args: Record<string, unknown>
  ): Promise<string>;

  /** Cancel a previously scheduled function. */
  cancel(scheduleId: string): Promise<void>;
}

// ---------------------------------------------------------------------------
// Context objects — what handlers receive
// ---------------------------------------------------------------------------

/** Context for query handlers (read-only). */
export interface QueryCtx {
  db: DbReader;
  auth: AuthInfo;
  /** Environment variables / secrets. */
  env: Record<string, string>;
}

/** Context for mutation handlers (read + write, transactional). */
export interface MutationCtx {
  db: DbWriter;
  auth: AuthInfo;
  stream: Stream;
  scheduler: Scheduler;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Create a typed error that triggers rollback. */
  error(code: string, message: string): Error;
}

/** Context for action handlers (external I/O, non-transactional). */
export interface ActionCtx {
  auth: AuthInfo;
  stream: Stream;
  scheduler: Scheduler;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Run a registered query within its own read transaction. */
  runQuery<T = unknown>(
    fnName: string,
    args: Record<string, unknown>
  ): Promise<T>;
  /** Run a registered mutation within its own write transaction. */
  runMutation<T = unknown>(
    fnName: string,
    args: Record<string, unknown>
  ): Promise<T>;
  /** Create a typed error. */
  error(code: string, message: string): Error;
  /**
   * HTTP request metadata — present only when the action was invoked via
   * a `defineRoute` HTTP binding. Missing when the action is called from
   * another action (`ctx.runAction`), a job, or the function dashboard.
   *
   * Use this to verify webhook signatures (Stripe, GitHub, Slack) that
   * require the raw request body — `rawBody` is the exact bytes the
   * signer signed, NOT the parsed JSON.
   *
   * ```ts
   * export default action({
   *   async handler(ctx) {
   *     const sig = ctx.request?.headers["stripe-signature"];
   *     stripe.webhooks.constructEvent(ctx.request!.rawBody, sig!, secret);
   *   },
   * });
   * ```
   */
  request?: RequestInfo;
}

/** HTTP request metadata available on an action's ctx when invoked via an
 *  HTTP route binding. Header names are lowercased. */
export interface RequestInfo {
  method: string;
  path: string;
  headers: Record<string, string>;
  rawBody: string;
}

// ---------------------------------------------------------------------------
// Function definition types
// ---------------------------------------------------------------------------

export type FnType = "query" | "mutation" | "action";

export interface FnDefinition<TArgs = unknown, TReturn = unknown> {
  type: FnType;
  args?: Record<string, Validator>;
  handler: (ctx: any, args: TArgs) => Promise<TReturn>;
}

// ---------------------------------------------------------------------------
// Validators
// ---------------------------------------------------------------------------

export interface Validator {
  type: string;
  optional?: boolean;
  /** For v.id("tableName") */
  table?: string;
  /** For v.array(v.string()) */
  items?: Validator;
  /** For v.object({...}) */
  fields?: Record<string, Validator>;
  /** For v.union(...) */
  variants?: Validator[];
  /** For v.literal("value") */
  value?: unknown;
}
