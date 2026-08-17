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
  /** Exact role slugs for the active organization/session. Custom roles do
   *  not imply `member` or `admin`; compare explicitly or use
   *  `ctx.requireMember` for an authoritative membership lookup. */
  roles: string[];
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
   * Exact k-nearest-neighbor search over a `field.vector(dims)` field.
   * Cosine similarity by default (`metric: "dot" | "l2"` to change);
   * hits come back best-first with the full row on `doc` (vector
   * fields stripped — re-fetch by id if you need the embedding).
   *
   * Available wherever `ctx.db` is — queries and mutations. Actions
   * have no `ctx.db`: embed there, then `ctx.runQuery` a query that
   * searches with the vector.
   *
   * ```ts
   * // In a query: the vector arrives as an arg (an action embedded it).
   * const { hits } = await ctx.db.vectorSearch("Doc", {
   *   field: "embedding",
   *   vector,
   *   limit: 5,
   *   filter: { status: "published" },   // equality / IN pre-filter
   * });
   * ```
   *
   * Exact scan, not ANN: every non-NULL embedding is scored. Fine to
   * ~100k rows per entity; past that, a dedicated vector store wins.
   * Throws `VECTOR_FIELD_NOT_FOUND` when `field` isn't a vector field
   * and `INVALID_QUERY` on dimension mismatch or bad filters.
   */
  vectorSearch(
    entity: string,
    query: VectorSearchQuery
  ): Promise<VectorSearchResult>;

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

/** Request shape for [`DbReader.vectorSearch`]. */
export interface VectorSearchQuery {
  /** The `vector(dims)` field to search. */
  field: string;
  /** Query embedding; length must equal the field's declared dims. */
  vector: number[];
  /** Max hits. Default 10, capped at 200. */
  limit?: number;
  /** Similarity metric. Default "cosine" (higher = closer);
   *  "dot" (higher = closer); "l2" (Euclidean distance, lower = closer). */
  metric?: "cosine" | "dot" | "l2";
  /** Equality pre-filter applied in SQL before scoring. A plain value
   *  means equality; an array means SQL `IN`; `null` means IS NULL. */
  filter?: Record<string, unknown>;
}

/** Result shape for [`DbReader.vectorSearch`]. */
export interface VectorSearchResult<T = Record<string, unknown>> {
  /** Best-first hits for the chosen metric. */
  hits: Array<{ id: string; score: number; doc: T }>;
  /** Milliseconds spent scanning + scoring. */
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

/**
 * Progressive output to the calling client (SSE). Every fn stream is
 * RESUMABLE: the host buffers each chunk under a server-assigned
 * stream id (the `X-Pylon-Stream-Id` response header) with a
 * monotonically increasing sequence, so a client that loses its
 * connection reconnects to `GET /api/fn-streams/<id>` from its last
 * cursor and misses nothing — including the terminal result after the
 * handler already returned. The handler never blocks on (or notices)
 * client disconnects; it just keeps writing.
 */
export interface Stream {
  /** Write a text chunk to the client (SSE). */
  write(data: string): void;

  /** Write a typed SSE event (`event: <name>` framing on the wire). */
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
 * (PYLON_EMAIL_PROVIDER env var → SendGrid / Resend / Stack0 /
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
 * supply the message. Failures are surfaced as thrown errors; on
 * success the return is void.
 *
 * Use cases: invite emails, password-reset hand-offs, notifications,
 * digest reports, calendar invites (attach an .ics with contentType
 * `text/calendar; method=REQUEST` and mail clients render an
 * RSVP-able event). NOT for marketing email — those should go
 * through a dedicated bulk transport, not the transactional path.
 */
export interface EmailAttachment {
  filename: string;
  /**
   * Full MIME content type, passed to the provider VERBATIM —
   * parameterized types like `text/calendar; method=REQUEST` are
   * preserved (that parameter is what makes an invite RSVP-able).
   */
  contentType: string;
  /** Base64-encoded file bytes. */
  content: string;
}

export interface EmailOptions {
  /** Single recipient address. */
  to: string;
  subject: string;
  /**
   * Plain-text body. Required even with `html` — it's the text/plain
   * part clients with HTML disabled fall back to.
   */
  text: string;
  /** Optional HTML body; providers send multipart with `text`. */
  html?: string;
  /**
   * Base64 attachments. Limits: at most 20 per email, 15MB of base64
   * text total (≈11MB of raw file data); larger sends throw before
   * any network I/O.
   */
  attachments?: EmailAttachment[];
}

export interface EmailSender {
  /** Send a plain-text email. `to` is a single address. */
  send(to: string, subject: string, body: string): Promise<void>;
  /** Send with options: HTML body and/or base64 attachments. */
  send(options: EmailOptions): Promise<void>;
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

  /**
   * Streaming completion. `onEvent` fires for each event as the
   * provider emits it; the promise resolves with the same assembled
   * response `complete` returns, so a tool-use loop can inspect
   * `stop_reason` after the text has already been streamed out.
   *
   * The typical agent shape pumps deltas straight to the client:
   *
   * ```ts
   * const res = await ctx.llm.stream(
   *   { messages, tools },
   *   (e) => { if (e.type === "text_delta") ctx.stream.write(e.text); },
   * );
   * if (res.stop_reason === "tool_use") { ...run tools, loop... }
   * ```
   *
   * Streaming does NOT extend the function's call deadline — it is an
   * absolute wall clock from invocation (PYLON_FN_CALL_TIMEOUT, 30s
   * default). A long agent run must declare its own `timeout` on the
   * function def.
   *
   * Same errors and same gating as {@link Llm.complete} — including
   * the model allowlist, so streaming can't be used to reach a model
   * `complete` would refuse.
   */
  stream(
    request: LlmCompleteRequest,
    onEvent: (event: LlmStreamEvent) => void,
  ): Promise<LlmCompleteResponse>;

  /**
   * Batch-embed texts via the configured embeddings provider. One
   * embedding per input, in input order. Pair with a
   * `field.vector(dims)` field and `ctx.db.vectorSearch` for
   * retrieval. From an action (which has no `ctx.db`), store via a
   * mutation:
   *
   * ```ts
   * const [vec] = await ctx.llm.embed([doc.body]);
   * await ctx.runMutation("saveEmbedding", { docId: doc.id, embedding: vec });
   * ```
   *
   * The embeddings provider is a separate axis from chat: with
   * `OPENAI_API_KEY` set it defaults to OpenAI
   * `text-embedding-3-small` (1536 dims) even when chat runs
   * Anthropic; `PYLON_EMBEDDINGS_PROVIDER=voyage` + `VOYAGE_API_KEY`
   * selects Voyage `voyage-3.5` (1024 dims).
   * `PYLON_EMBEDDINGS_MODEL` overrides the model.
   *
   * Not available in queries (reactive re-runs would re-bill the
   * provider) — embed in a mutation/action and store the vector.
   * Errors carry `err.code`: `EMBEDDINGS_NOT_CONFIGURED`,
   * `PROVIDER_HTTP_<code>`, `PROVIDER_UNREACHABLE`,
   * `INVALID_REQUEST`.
   */
  embed(input: string[], opts?: { model?: string }): Promise<number[][]>;
}

/**
 * One event from an in-flight {@link Llm.stream} call.
 *
 * `tool_use_start` opens a tool call; the `tool_input_delta` events
 * that follow carry its arguments as raw JSON fragments — concatenate
 * them and parse once, rather than parsing each fragment. `done`
 * always fires last, including on a partial failure.
 */
export type LlmStreamEvent =
  | { type: "text_delta"; text: string }
  | { type: "tool_use_start"; id: string; name: string }
  | { type: "tool_input_delta"; partial_json: string }
  | {
      type: "done";
      stop_reason: string;
      usage: { input_tokens: number; output_tokens: number };
    };

/**
 * Server-originated realtime push. Broadcasts an event to every
 * subscriber of a presence room — the same rooms clients join with
 * `useRoom(roomId, userId)`, and the same delivery path a member's
 * `broadcast()` uses.
 *
 * This is the surface for fanning agent output out to a second device
 * or a second tab that is CONNECTED RIGHT NOW: write tokens to the
 * room and every current watcher gets them, not just the caller
 * holding the HTTP response. Delivery is live-only — a subscriber that
 * reconnects does NOT replay messages sent during its gap. For output
 * that must survive a closed tab or a dropped connection, rely on the
 * fn stream itself: every `ctx.stream` stream is buffered server-side
 * and resumable by stream id (`streamFn`'s `onStreamId` +
 * `resumeStream` in the clients), including the final result after the
 * handler finished.
 *
 * Not available in queries — a reactive handler re-runs on every dep
 * change, which would re-broadcast each time.
 */
export interface Rooms {
  /**
   * Push `data` to every subscriber of `room` under `topic`.
   * Resolves `{ delivered: false }` when the room has no members —
   * broadcasting into an empty room is a no-op, not an error, so an
   * agent doesn't need to know whether anyone is watching.
   */
  broadcast(
    room: string,
    topic: string,
    data?: unknown,
  ): Promise<{ delivered: boolean }>;
}

/**
 * Durable workflows, driven from app code (`ctx.workflows` on mutations
 * and actions — not queries, whose reactive re-runs would re-start).
 * Workflows are declared in the app's `workflows/` directory; see the
 * `workflow()` export.
 */
export interface Workflows {
  /**
   * Start a workflow instance by name. Returns immediately with the
   * instance id — the engine's background driver executes the steps.
   */
  start(name: string, input?: unknown): Promise<{ id: string }>;
  /**
   * Deliver an event to an instance paused on
   * `wf.waitForEvent(event)`. Rejects when the instance isn't waiting
   * for that event.
   */
  sendEvent(
    workflowId: string,
    event: string,
    data?: unknown,
  ): Promise<{ delivered: boolean }>;
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

/**
 * Signed file-download URLs. `GET /api/files/<id>` only serves a file to its
 * owner (or an unscoped admin) — `signedUrl` is the supported way to
 * authorize a cross-user read: the host mints a short-lived HMAC-signed
 * path (`/api/files/<id>?sig=...&exp=...`) that the GET handler honors as
 * an alternative to the owner check, anonymously fetchable (works in
 * `<img src>` and download links).
 *
 * WHO gets a URL is the calling function's responsibility — wrap the mint
 * in a membership-gated function so authorization stays app-policy-driven:
 *
 * ```ts
 * export default query({
 *   args: { eventId: v.id("Event"), fileId: v.string() },
 *   async handler(ctx, args) {
 *     const event = await ctx.db.get("Event", args.eventId);
 *     await ctx.requireMember(event.orgId, { role: "organizer" });
 *     return { url: await ctx.files.signedUrl(args.fileId) };
 *   },
 * });
 * ```
 */
export interface Files {
  /**
   * Mint a signed download path for a file id. `ttlSecs` defaults to 300
   * and is capped by the host (24h) — a signed URL is a bearer capability,
   * so keep lifetimes short.
   */
  signedUrl(fileId: string, opts?: { ttlSecs?: number }): Promise<string>;
}

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
  /**
   * Create a typed error. Unlike the mutation version there's no
   * transaction to roll back — this exists so a query can answer with a
   * code the caller can branch on (`AUCTION_NOT_FOUND`) instead of a bare
   * `throw new Error(...)`, which reaches the client as `UNKNOWN`.
   */
  error(code: string, message: string): Error;
  /** Assert org membership (optionally a role) — see {@link RequireMember}. */
  requireMember: RequireMember;
  /** Signed file-download URLs — see {@link Files}. */
  files: Files;
  /**
   * Fires when the host cancels this call (idle timeout exceeded).
   * Thread it into `fetch(url, { signal: ctx.signal })` or SDK calls so
   * a cancelled call stops its outbound work too — the runtime already
   * makes every later `ctx.*` call throw `CALL_CANCELLED`. Optional
   * because older hosts don't send cancel frames.
   */
  signal?: AbortSignal;
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
  /** Server-originated realtime push — see {@link Rooms}. */
  rooms: Rooms;
  /** Per-user OAuth connection registry. */
  connections: Connections;
  /** Durable workflows: start / deliver events — see {@link Workflows}. */
  workflows: Workflows;
  /** Signed file-download URLs — see {@link Files}. */
  files: Files;
  /** Create a typed error that triggers rollback. */
  error(code: string, message: string): Error;
  /** Assert org membership (optionally a role) — see {@link RequireMember}. */
  requireMember: RequireMember;
  /**
   * Fires when the host cancels this call (idle timeout exceeded).
   * Thread it into `fetch(url, { signal: ctx.signal })` or SDK calls so
   * a cancelled call stops its outbound work too — the runtime already
   * makes every later `ctx.*` call throw `CALL_CANCELLED`. Optional
   * because older hosts don't send cancel frames.
   */
  signal?: AbortSignal;
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
  /** Server-originated realtime push — see {@link Rooms}. */
  rooms: Rooms;
  /** Per-user OAuth connection registry. */
  connections: Connections;
  /** Durable workflows: start / deliver events — see {@link Workflows}. */
  workflows: Workflows;
  /** Environment variables / secrets. */
  env: Record<string, string>;
  /** Signed file-download URLs — see {@link Files}. */
  files: Files;
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
   * Fires when the host cancels this call (idle timeout exceeded).
   * Thread it into `fetch(url, { signal: ctx.signal })` or SDK calls so
   * a cancelled call stops its outbound work too — the runtime already
   * makes every later `ctx.*` call throw `CALL_CANCELLED`. Optional
   * because older hosts don't send cancel frames.
   */
  signal?: AbortSignal;
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

declare const validatorType: unique symbol;
declare const validatorOptional: unique symbol;

/**
 * Runtime validator carrying its validated TypeScript type.
 *
 * The symbol properties are type-only markers: they emit no runtime data and
 * let the function constructors derive handler arguments from an `args` schema.
 */
export interface Validator<
  T = unknown,
  TOptional extends boolean = boolean,
> {
  type: string;
  optional?: boolean;
  readonly [validatorType]?: T;
  readonly [validatorOptional]?: TOptional;
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

/** Any validator accepted in an argument schema. */
export type AnyValidator = Validator<any, boolean>;

/** A function argument schema keyed by argument name. */
export type ValidatorSchema = Record<string, AnyValidator>;

/** Extract the value accepted by a validator. */
export type InferValidator<TValidator extends AnyValidator> =
  TValidator extends Validator<infer TValue, boolean> ? TValue : never;

type OptionalValidatorKeys<TSchema extends ValidatorSchema> = {
  [TKey in keyof TSchema]-?: TSchema[TKey] extends Validator<any, true>
    ? TKey
    : never;
}[keyof TSchema];

type RequiredValidatorKeys<TSchema extends ValidatorSchema> = Exclude<
  keyof TSchema,
  OptionalValidatorKeys<TSchema>
>;

/** Derive a handler argument object from a validator schema. */
export type InferArgs<TSchema extends ValidatorSchema> = {
  [TKey in RequiredValidatorKeys<TSchema>]: InferValidator<TSchema[TKey]>;
} & {
  [TKey in OptionalValidatorKeys<TSchema>]?: InferValidator<TSchema[TKey]>;
};
