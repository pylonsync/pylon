/**
 * Function definition constructors.
 *
 * These are the primary API for defining server-side functions.
 *
 * Each constructor (query / mutation / action) has two overloads:
 *
 *   1. `auth` omitted, or `auth: "user" | "admin"` →  the framework
 *      enforces a real signed-in user, so the handler's
 *      `ctx.auth.userId` is narrowed from `string | null` to
 *      `string`. The redundant `if (!ctx.auth.userId) throw …`
 *      check disappears from app code.
 *
 *   2. `auth: "public" | "guest"` →  the function is reachable by
 *      anonymous callers, so `ctx.auth.userId` stays `string | null`
 *      and the handler must check it manually if relevant.
 *
 * The framework gate happens BEFORE the handler runs (in Rust,
 * inside the router). A handler can't accidentally leak data by
 * forgetting an auth check — when `auth: "user"` is in effect,
 * an anonymous request never reaches the handler at all.
 */

import type {
  FnDefinition,
  QueryCtx,
  MutationCtx,
  ActionCtx,
  ValidatorSchema,
  InferArgs,
  AuthMode,
} from "./types";

/** Default auth mode applied when a definition omits `auth`. */
const DEFAULT_AUTH: AuthMode = "user";

interface CommonDef<
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> {
  args?: TSchema;
  /**
   * When true, the function is callable only via `ctx.runQuery()` /
   * `ctx.runMutation()` / `ctx.runAction()` from another function
   * — never via the public `/api/fn/<name>` HTTP endpoint. The
   * router refuses external calls with `404 FN_NOT_FOUND` so
   * probing can't even confirm the name exists.
   *
   * Use for helper functions that are safe inside trusted
   * wrappers but unsafe if any caller could invoke them directly
   * (e.g. they trust args without re-checking caller authority).
   *
   * Internal functions inherit the wrapping handler's auth — the
   * `auth` field has no effect when `internal: true`. The router
   * isn't reachable anyway, and the caller has already passed its
   * own gate.
   */
  internal?: boolean;
  /**
   * IDLE-timeout SECONDS for this function: how long it may go without
   * producing any activity (a stream chunk, a `ctx.db` op, an LLM
   * event) before the host cancels the call. Defaults to
   * `PYLON_FN_CALL_TIMEOUT` (30s). Activity restarts the budget, so a
   * streaming agent run stays alive as long as it keeps producing; a
   * silent hang is cancelled at the budget. Total lifetime is capped at
   * 10× this value however chatty the call is.
   *
   * Cancellation is per-call: the handler's next `ctx.*` call throws
   * `CALL_CANCELLED` and `ctx.signal` aborts — other in-flight calls on
   * the same worker are untouched. Raise it for legitimately long
   * SILENT work (a single slow external call, synchronous CPU work);
   * this also lifts the runtime's wedge backstop for the worker while
   * such a call is in flight, so a busy-but-progressing worker (e.g.
   * one doing synchronous canvas/image work that blocks the event loop)
   * isn't respawned out from under the work.
   */
  timeout?: number;
}

interface QueryDefRequired<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  /** Defaults to `"user"`. See [`AuthMode`] for the full surface. */
  auth?: "user" | "admin";
  handler: (
    ctx: QueryCtx<"required">,
    args: TArgs,
  ) => Promise<TReturn>;
}

interface QueryDefOptional<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  auth: "public" | "guest";
  handler: (
    ctx: QueryCtx<"optional">,
    args: TArgs,
  ) => Promise<TReturn>;
}

interface MutationDefRequired<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  auth?: "user" | "admin";
  handler: (
    ctx: MutationCtx<"required">,
    args: TArgs,
  ) => Promise<TReturn>;
}

interface MutationDefOptional<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  auth: "public" | "guest";
  handler: (
    ctx: MutationCtx<"optional">,
    args: TArgs,
  ) => Promise<TReturn>;
}

interface ActionDefRequired<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  auth?: "user" | "admin";
  handler: (
    ctx: ActionCtx<"required">,
    args: TArgs,
  ) => Promise<TReturn>;
}

interface ActionDefOptional<
  TArgs,
  TReturn,
  TSchema extends ValidatorSchema | undefined = ValidatorSchema | undefined,
> extends CommonDef<TSchema> {
  auth: "public" | "guest";
  handler: (
    ctx: ActionCtx<"optional">,
    args: TArgs,
  ) => Promise<TReturn>;
}

/**
 * Define a read-only query function.
 *
 * Queries use the read pool — they never block writes and can run
 * concurrently. They cannot modify data.
 *
 * @example
 * ```typescript
 * export default query({
 *   // auth: "user" is the default — ctx.auth.userId is `string`,
 *   // not `string | null`, inside the handler.
 *   args: { auctionId: v.string() },
 *   async handler(ctx, args) {
 *     return ctx.db.query("Lot", {
 *       auctionId: args.auctionId,
 *       authorId: ctx.auth.userId,
 *     });
 *   },
 * });
 * ```
 *
 * @example
 * ```typescript
 * // Explicitly public — landing-page count, never auth-shaped.
 * export default query({
 *   auth: "public",
 *   async handler(ctx) {
 *     return ctx.db.query("PublicPost", { published: true });
 *   },
 * });
 * ```
 */
export function query<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "user" | "admin" | undefined,
>(
  def: QueryDefRequired<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth?: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function query<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "public" | "guest",
>(
  def: QueryDefOptional<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function query<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: QueryDefRequired<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function query<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: QueryDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function query<TArgs, TReturn>(
  def:
    | QueryDefRequired<TArgs, TReturn>
    | QueryDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn> {
  return {
    type: "query",
    args: def.args,
    handler: def.handler as FnDefinition<TArgs, TReturn>["handler"],
    internal: def.internal,
    auth: def.auth ?? DEFAULT_AUTH,
    timeout: def.timeout,
  };
}

/**
 * Define a transactional mutation function.
 *
 * The entire handler IS the transaction. If it returns, all writes commit
 * atomically. If it throws, all writes roll back — including scheduled
 * functions.
 *
 * Mutations can stream data to the client via `ctx.stream.write()`.
 * Stream chunks are sent immediately; DB writes commit at the end.
 *
 * @example
 * ```typescript
 * export default mutation({
 *   args: { lotId: v.string(), amount: v.number() },
 *   async handler(ctx, args) {
 *     // ctx.auth.userId is `string` (not nullable) — the runtime
 *     // already enforced `auth: "user"` (default) before reaching here.
 *     const lot = await ctx.db.get("Lot", args.lotId);
 *     if (!lot) throw ctx.error("NOT_FOUND", "Lot not found");
 *     await ctx.db.insert("Bid", {
 *       lotId: args.lotId,
 *       amount: args.amount,
 *       bidderId: ctx.auth.userId,
 *     });
 *     return { accepted: true };
 *   },
 * });
 * ```
 */
export function mutation<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "user" | "admin" | undefined,
>(
  def: MutationDefRequired<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth?: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function mutation<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "public" | "guest",
>(
  def: MutationDefOptional<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function mutation<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: MutationDefRequired<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function mutation<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: MutationDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function mutation<TArgs, TReturn>(
  def:
    | MutationDefRequired<TArgs, TReturn>
    | MutationDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn> {
  return {
    type: "mutation",
    args: def.args,
    handler: def.handler as FnDefinition<TArgs, TReturn>["handler"],
    internal: def.internal,
    auth: def.auth ?? DEFAULT_AUTH,
    timeout: def.timeout,
  };
}

/**
 * Define an action function (external I/O allowed).
 *
 * Actions can call external APIs (fetch, email, Stripe, etc.) but cannot
 * access the database directly. Use `ctx.runQuery()` and `ctx.runMutation()`
 * for DB access — each runs in its own transaction.
 *
 * Actions are NOT automatically retried because they may have side effects.
 *
 * **Important:** policies don't gate actions — `auth: "user"` (the
 * default) is the only thing protecting an action from anonymous calls.
 * An action that charges Stripe, hits a private API, or reads a
 * secret will respond happily to anonymous POSTs unless this gate
 * fires. Pick `auth: "public"` explicitly only when the endpoint is
 * meant to be open (a webhook receiver, a public form submit, …).
 *
 * @example
 * ```typescript
 * export default action({
 *   args: { lotId: v.string() },
 *   async handler(ctx, args) {
 *     const lot = await ctx.runQuery("lotDetails", { lotId: args.lotId });
 *     await fetch("https://api.sendgrid.com/...", { ... });
 *     await ctx.runMutation("markNotified", { lotId: args.lotId });
 *   },
 * });
 * ```
 *
 * @example
 * ```typescript
 * // GitHub webhook receiver — public because GitHub doesn't sign
 * // in, the handler verifies the signature itself, then optionally
 * // elevates.
 * export default action({
 *   auth: "public",
 *   async handler(ctx) {
 *     const ok = verifyGithubSignature(secret, ctx.request!.rawBody, sig);
 *     if (!ok) throw ctx.error("INVALID_SIGNATURE", "bad sig");
 *     await ctx.auth.elevate({ admin: true, reason: "github webhook hmac" });
 *     // …
 *   },
 * });
 * ```
 */
export function action<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "user" | "admin" | undefined,
>(
  def: ActionDefRequired<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth?: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function action<
  const TSchema extends ValidatorSchema,
  TReturn,
  TAuth extends "public" | "guest",
>(
  def: ActionDefOptional<InferArgs<TSchema>, TReturn, TSchema> & {
    args: TSchema;
    auth: TAuth;
  },
): FnDefinition<InferArgs<TSchema>, TReturn>;
export function action<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: ActionDefRequired<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function action<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: ActionDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn>;
export function action<TArgs, TReturn>(
  def:
    | ActionDefRequired<TArgs, TReturn>
    | ActionDefOptional<TArgs, TReturn>,
): FnDefinition<TArgs, TReturn> {
  return {
    type: "action",
    args: def.args,
    handler: def.handler as FnDefinition<TArgs, TReturn>["handler"],
    internal: def.internal,
    auth: def.auth ?? DEFAULT_AUTH,
    timeout: def.timeout,
  };
}
