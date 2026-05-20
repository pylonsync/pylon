/**
 * Type definitions for the function system.
 */

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/**
 * Declarative auth requirement for a function. The framework
 * enforces this BEFORE the handler runs — if the caller doesn't
 * meet the bar, the request rejects with a typed error and the
 * handler is never invoked.
 *
 * Functions default to `"user"` (signed-in required) when this
 * field is omitted. That's the secure-by-default position: a
 * forgotten `if (!ctx.auth.userId)` check never leaks data,
 * because the runtime made the check before the handler ran.
 *
 * Modes:
 * - `"public"` — anyone, including unauthenticated callers. Use
 *   for healthchecks, landing-page form submits, intentionally-open
 *   webhooks. Must be explicit; never the default.
 * - `"guest"` — anonymous-with-stable-id sessions count, plus
 *   any authenticated user. Use for cart-style pre-login state.
 * - `"user"` — a real signed-in user (default). Guest sessions
 *   are rejected. Inside the handler, `ctx.auth.userId` is
 *   narrowed from `string | null` to `string` so the redundant
 *   null check can be dropped.
 * - `"admin"` — `ctx.auth.isAdmin === true`. Use for ops
 *   endpoints exposed via `/api/fn/...`.
 */
export type AuthMode = "public" | "guest" | "user" | "admin";

/**
 * `userId` shape narrows based on the function's declared auth
 * requirement. `auth: "user"` and `auth: "admin"` both guarantee
 * a real signed-in user, so the handler sees a non-null string.
 * `auth: "public"` and `auth: "guest"` allow anonymous callers,
 * so the handler must keep checking.
 */
export type AuthRequirement = "required" | "optional";

export interface AuthInfo<R extends AuthRequirement = "optional"> {
  userId: R extends "required" ? string : string | null;
  isAdmin: boolean;
  /** Active tenant id (selected organization) for multi-tenant apps.
   *  Null when the session hasn't selected one. */
  tenantId: string | null;
  /**
   * Promote the call's auth context after the handler has done its
   * own authentication check (HMAC signature verification on a
   * webhook, JWT validation, custom token check). Used by webhook
   * receivers — they're necessarily public (external systems POST
   * to them) but want to schedule internal:true workers after
   * they've proven the request came from a trusted source.
   *
   * The framework does NOT verify the developer actually checked
   * anything before calling this — that's on you. The `reason` is
   * mandatory and gets logged at INFO with the function name so
   * every elevation is auditable.
   *
   * ```ts
   * // Github webhook example:
   * const ok = await verifyGithubSignature(secret, rawBody, sig);
   * if (!ok) throw ctx.error("INVALID_SIGNATURE", "bad sig");
   * await ctx.auth.elevate({
   *   admin: true,
   *   reason: "github webhook hmac verified",
   * });
   * // Now this works — caller_is_admin=true for the gate:
   * await ctx.scheduler.runAfter(0, "deployProject", { deploymentId });
   * ```
   *
   * After calling `elevate({ admin: true })`, `auth.isAdmin` is also
   * mutated to true locally so subsequent reads in the same handler
   * see the new value.
   */
  elevate(options: { admin: boolean; reason: string }): Promise<void>;
}

// ---------------------------------------------------------------------------
// Database — read operations
// ---------------------------------------------------------------------------

export interface DbReader {
  /**
   * Escape hatch: same surface as `ctx.db` but operations bypass
   * the framework's caller-aware policy gate (gated by
   * `PYLON_STRICT_FN_POLICIES=1`, Phase 2). Use sparingly, only
   * in code that runs from a trusted server-internal context —
   * webhook receivers (after signature verification), scheduled
   * cron sweeps, admin tooling. The plain `ctx.db.*` reads
   * already work for the caller's-own-data case; `ctx.db.unsafe`
   * is the answer when you genuinely need cross-tenant or
   * cross-user reads.
   *
   * Every call should carry a justifying comment per codebase
   * convention. A future `pylon lint` rule will flag bare
   * `ctx.db.unsafe.*` without a comment immediately above.
   *
   * Optional on the type because old Pylon runtimes don't ship
   * it; new code that targets v0.3.161+ can rely on the field.
   */
  unsafe?: DbReader;

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
  /**
   * Escape hatch — same shape as [`DbReader.unsafe`] but with the
   * write surface (insert/update/delete/link/unlink/advisoryLock).
   * Overrides the inherited read-only `unsafe` from DbReader.
   */
  unsafe?: DbWriter;

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

  /**
   * Acquire a transaction-scoped advisory lock on `key`. Held until
   * the mutation tx commits or rolls back. Two concurrent mutations
   * holding the same key serialize on Postgres; on SQLite this is a
   * noop because writers are already serialized at the connection
   * level.
   *
   * Use this to close TOCTOU windows on quota / uniqueness checks:
   * call `advisoryLock` BEFORE the count query so the second tx
   * blocks on the first's commit before observing state.
   *
   * Example:
   * ```ts
   * await ctx.db.advisoryLock(`org_count:${ctx.auth.userId}`);
   * const orgs = await ctx.db.query("Organization", { createdBy: userId });
   * if (orgs.length >= cap) throw ctx.error("QUOTA_EXCEEDED", "...");
   * await ctx.db.insert("Organization", { ... });
   * ```
   */
  advisoryLock(key: string): Promise<void>;
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

/**
 * Transactional email transport.
 *
 * Sends through whatever provider the runtime is configured for
 * (PYLON_EMAIL_PROVIDER env var → SendGrid / Resend / Stack0 / SMTP /
 * webhook). Available on action ctx only — sending email is external
 * I/O, not allowed in mutation transactions.
 *
 * The runtime owns provider config + credentials; functions only
 * supply the (to, subject, body) tuple. Failures are surfaced as
 * thrown errors; on success the return is void.
 *
 * Use cases: invite emails, password-reset hand-offs, notifications,
 * digest reports. NOT for marketing email — those should go through
 * a dedicated bulk transport, not the transactional path.
 */
export interface EmailSender {
  /** Send a plain-text email. `to` is a single address. */
  send(to: string, subject: string, body: string): Promise<void>;
}

// ---------------------------------------------------------------------------
// Context objects — what handlers receive
// ---------------------------------------------------------------------------

/** Context for query handlers (read-only). */
export interface QueryCtx<R extends AuthRequirement = "optional"> {
  db: DbReader;
  auth: AuthInfo<R>;
  /** Environment variables / secrets. */
  env: Record<string, string>;
}

/** Context for mutation handlers (read + write, transactional). */
export interface MutationCtx<R extends AuthRequirement = "optional"> {
  db: DbWriter;
  auth: AuthInfo<R>;
  stream: Stream;
  scheduler: Scheduler;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Create a typed error that triggers rollback. */
  error(code: string, message: string): Error;
}

/** Context for action handlers (external I/O, non-transactional). */
export interface ActionCtx<R extends AuthRequirement = "optional"> {
  auth: AuthInfo<R>;
  stream: Stream;
  scheduler: Scheduler;
  /** Send transactional email via the runtime's configured provider. */
  email: EmailSender;
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
  /**
   * When true, this function is reachable only via `ctx.runQuery()` /
   * `ctx.runMutation()` / `ctx.runAction()` from another function —
   * the public `/api/fn/<name>` endpoint refuses external calls.
   * The router enforces this; the runtime treats internal == external
   * for execution.
   */
  internal?: boolean;
  /**
   * Auth requirement enforced by the runtime before the handler is
   * invoked. Defaults to `"user"` — every function is signed-in only
   * unless explicitly opted out via `auth: "public"`. See [`AuthMode`].
   */
  auth?: AuthMode;
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
