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
   * Required on the type (every runtime since v0.3.161 ships it) —
   * but absent on the unsafe surface itself, so `ctx.db.unsafe.unsafe`
   * is a compile error rather than a runtime undefined.
   */
  unsafe: Omit<DbReader, "unsafe">;

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
  unsafe: Omit<DbWriter, "unsafe">;

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
 * This is the APP email channel (`PYLON_EMAIL_*`): arbitrary recipient
 * and body, so it must be the app's own provider. It is deliberately
 * separate from Pylon's built-in auth emails (codes / password reset /
 * invitations), which send via a `PYLON_AUTH_EMAIL_*` channel. On Pylon
 * Cloud the auth channel may be a shared, locked-down platform key, so
 * `ctx.email` stays inert until you set `PYLON_EMAIL_*` yourself — the
 * shared auth key can never be used to send arbitrary mail.
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
// LLM — provider-abstracted text/tool-use completion
// ---------------------------------------------------------------------------

/**
 * Server-side LLM client. Available on every ctx variant (query,
 * mutation, action) because agent loops often run as queries —
 * read tool args from the message, ship the response back.
 *
 * Provider is configured at the server boot (PYLON_LLM_PROVIDER +
 * ANTHROPIC_API_KEY or OPENAI_API_KEY). The wire shape is Anthropic
 * Messages — OpenAI calls translate at the transport boundary, so
 * the same caller code works against either provider.
 *
 * The framework does NOT expose this surface to the browser; clients
 * that need streaming should call POST /api/ai/stream directly.
 * `ctx.llm.complete` is server-only on purpose — the API key never
 * leaves the runtime process.
 */
export interface Llm {
  /**
   * Send a completion request to the configured LLM provider. The
   * shape is Anthropic Messages: a list of {role, content} pairs
   * where content is either a string or a list of content blocks
   * (text, tool_use, tool_result). Returns the full response once
   * the model finishes generating.
   *
   * For agent tool-use loops, inspect `response.stopReason` — when
   * it's `"tool_use"`, append the assistant's content (which
   * includes the `tool_use` blocks) plus your `tool_result`
   * follow-ups to the message list and call again. Loop until
   * `stopReason === "end_turn"`.
   *
   * Errors are thrown as standard Error objects with an `err.code`
   * property set to one of: `LLM_NOT_CONFIGURED`, `MODEL_NOT_ALLOWED`,
   * `MODEL_OVERRIDE_FORBIDDEN`, `PROVIDER_HTTP_<code>`,
   * `PROVIDER_UNREACHABLE`, `INVALID_REQUEST`.
   */
  complete(request: LlmCompleteRequest): Promise<LlmCompleteResponse>;
}

export interface LlmMessage {
  role: "user" | "assistant" | "system" | "tool";
  content: string | LlmContentBlock[];
}

export type LlmContentBlock =
  | { type: "text"; text: string }
  | {
      type: "tool_use";
      id: string;
      name: string;
      input: Record<string, unknown>;
    }
  | {
      type: "tool_result";
      tool_use_id: string;
      content: string;
      is_error?: boolean;
    };

export interface LlmTool {
  name: string;
  description?: string;
  /** JSON Schema object describing the tool's input shape. */
  input_schema: Record<string, unknown>;
}

export interface LlmCompleteRequest {
  /** Override the server's default model. Subject to
   *  PYLON_AI_MODELS_ALLOWED gating for non-admin callers. */
  model?: string;
  messages: LlmMessage[];
  system?: string;
  tools?: LlmTool[];
  /** Defaults to 4096. */
  max_tokens?: number;
  temperature?: number;
}

export interface LlmCompleteResponse {
  model: string;
  content: LlmContentBlock[];
  /** `end_turn` | `tool_use` | `max_tokens` | `stop_sequence` */
  stop_reason: string;
  usage: { input_tokens: number; output_tokens: number };
}

// ---------------------------------------------------------------------------
// Connections — per-user OAuth integrations
// ---------------------------------------------------------------------------

/**
 * Server-side OAuth connection registry. Apps declare connections
 * via `defineConnection({...})` in `app.ts`; this surface lets
 * actions fetch fresh access tokens (auto-refresh) and start the
 * OAuth dance.
 *
 * Available on mutation + action ctx only — connections perform
 * external I/O (token refresh, DB writes) that doesn't belong
 * inside a reactive query.
 *
 * All ops require an authenticated caller (`ctx.auth.userId !==
 * null`). Public functions must `ctx.auth.elevate({ admin: true,
 * reason: "..." })` before reaching `ctx.connections.*`.
 */
export interface Connections {
  /**
   * Mint the URL the browser should navigate to so the user can
   * link an external account. `name` matches a `defineConnection({...})`
   * entry. `postRedirect` (optional) is where the browser lands
   * after a successful callback — defaults to `/`.
   *
   * Throws `CONNECTIONS_NOT_CONFIGURED`, `CONNECTION_UNKNOWN`,
   * `PROVIDER_NOT_CONFIGURED`, or `ENCRYPTION_REQUIRED` (refresh
   * tokens are not allowed to land in plaintext).
   */
  authorizeUrl(
    name: string,
    opts?: { postRedirect?: string }
  ): Promise<{ url: string }>;

  /**
   * Returns a fresh access token for `(ctx.auth.userId, name)`. If
   * the stored token expires within 60s, the framework refreshes
   * via the provider's refresh-token grant FIRST, persists the new
   * token pair, then returns the new access token.
   *
   * Throws `CONNECTION_NOT_LINKED` when the user hasn't started
   * the OAuth flow, `REFRESH_FAILED` when the provider rejects
   * the refresh token (user must re-link).
   */
  get(name: string): Promise<{
    accessToken: string;
    scope: string | null;
    expiresAt: number | null;
  }>;

  /** List the signed-in user's linked connections. Token values
   *  are NOT included — call `get(name)` for those. */
  list(): Promise<{
    connections: Array<{
      name: string;
      provider: string;
      scope: string | null;
      expiresAt: number | null;
      updatedAt: number;
    }>;
  }>;

  /** Remove the stored connection. Provider-side revocation is
   *  the caller's responsibility — most providers expose a separate
   *  `/revoke` endpoint that this surface intentionally doesn't
   *  call (revoke vs unlink semantics differ per provider). */
  disconnect(name: string): Promise<{ disconnected: boolean }>;
}

// ---------------------------------------------------------------------------
// Context objects — what handlers receive
// ---------------------------------------------------------------------------

/** Options for `ctx.requireMember()`. */
export interface RequireMemberOptions {
  /**
   * Allowed role(s). The caller's membership role must be one of these.
   * Omit to require ANY membership regardless of role.
   */
  role?: string | string[];
  /**
   * The membership entity to check. Default `"OrgMember"` — the same entity
   * the framework's org/tenant machinery uses. Override for a custom model.
   */
  entity?: string;
  /** Field on the membership entity holding the org/tenant id. Default `"orgId"`. */
  orgField?: string;
  /** Field holding the user id. Default `"userId"`. */
  userField?: string;
  /** Field holding the role. Default `"role"`. */
  roleField?: string;
}

/** The membership row returned by `ctx.requireMember()`. */
export type MemberRow = Record<string, unknown> & { role?: string };

/**
 * Assert the caller is a member of `orgId` (optionally with one of `role`),
 * returning the membership row. Throws a typed error otherwise:
 * `UNAUTHENTICATED` (no signed-in user), `MISSING_ORG` (no orgId), or
 * `FORBIDDEN` (not a member / wrong role).
 *
 * This is the authoritative authorization gate for org-scoped writes —
 * actions + mutations BYPASS entity read policies, so a function that trusts
 * an attacker-supplied `orgId`/`projectId` is an IDOR unless it re-checks
 * membership. `requireMember` makes the safe path the default path.
 *
 * ```ts
 * export default mutation({
 *   args: { orgId: v.id("Organization"), name: v.string() },
 *   async handler(ctx, args) {
 *     await ctx.requireMember(args.orgId, { role: ["owner", "admin"] });
 *     // …safe to mutate org-scoped data now…
 *   },
 * });
 * ```
 *
 * The membership entity must let the caller read their OWN membership row
 * (the standard `auth.userId == data.userId` read policy) — the check runs
 * with the caller's identity.
 */
export type RequireMember = (
  orgId: string,
  opts?: RequireMemberOptions,
) => Promise<MemberRow>;

/** Context for query handlers (read-only).
 *
 * NOTE: `ctx.llm` is NOT exposed here. Queries are reactive: a
 * subscribed query re-runs whenever its `ctx.db.*` reads change.
 * Calling a stochastic, paid LLM from a query would (a) silently
 * burn the framework's API key on every dep invalidation, and
 * (b) violate the reactive purity contract (same inputs → same
 * outputs). LLM calls belong in mutations (transactional) or
 * actions (external I/O). */
export interface QueryCtx<R extends AuthRequirement = "optional"> {
  db: DbReader;
  auth: AuthInfo<R>;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Assert org membership (optionally a role) — see {@link RequireMember}. */
  requireMember: RequireMember;
}

/** Context for mutation handlers (read + write, transactional). */
export interface MutationCtx<R extends AuthRequirement = "optional"> {
  db: DbWriter;
  auth: AuthInfo<R>;
  stream: Stream;
  scheduler: Scheduler;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Provider-abstracted LLM client. */
  llm: Llm;
  /** Per-user OAuth connection registry. */
  connections: Connections;
  /** Create a typed error that triggers rollback. */
  error(code: string, message: string): Error;
  /** Assert org membership (optionally a role) — see {@link RequireMember}. */
  requireMember: RequireMember;
}

/** Context for action handlers (external I/O, non-transactional). */
export interface ActionCtx<R extends AuthRequirement = "optional"> {
  auth: AuthInfo<R>;
  stream: Stream;
  scheduler: Scheduler;
  /** Send transactional email via the runtime's configured provider. */
  email: EmailSender;
  /** Provider-abstracted LLM client. */
  llm: Llm;
  /** Per-user OAuth connection registry. */
  connections: Connections;
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
  /** Assert org membership (optionally a role) — see {@link RequireMember}. */
  requireMember: RequireMember;
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
  /**
   * Max wall-clock seconds this function may run before the runtime
   * recycles its worker. Defaults to `PYLON_FN_CALL_TIMEOUT` (30s).
   * Raise for legitimately long-running work; also lifts the wedge
   * backstop while the call is in flight. See the `timeout` option docs.
   */
  timeout?: number;
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
