/**
 * Function definition constructors.
 *
 * These are the primary API for defining server-side functions.
 */

import type {
  FnDefinition,
  QueryCtx,
  MutationCtx,
  ActionCtx,
  Validator,
} from "./types";

interface QueryDef<TArgs, TReturn> {
  args?: Record<string, Validator>;
  handler: (ctx: QueryCtx, args: TArgs) => Promise<TReturn>;
}

interface MutationDef<TArgs, TReturn> {
  args?: Record<string, Validator>;
  handler: (ctx: MutationCtx, args: TArgs) => Promise<TReturn>;
}

interface ActionDef<TArgs, TReturn> {
  args?: Record<string, Validator>;
  handler: (ctx: ActionCtx, args: TArgs) => Promise<TReturn>;
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
 *   args: { auctionId: v.string() },
 *   async handler(ctx, args) {
 *     return ctx.db.query("Lot", {
 *       auctionId: args.auctionId,
 *       $order: { closesAt: "asc" },
 *     });
 *   },
 * });
 * ```
 */
export function query<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: QueryDef<TArgs, TReturn>
): FnDefinition<TArgs, TReturn> {
  return { type: "query", args: def.args, handler: def.handler };
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
 *     const lot = await ctx.db.get("Lot", args.lotId);
 *     if (!lot) throw ctx.error("NOT_FOUND", "Lot not found");
 *     await ctx.db.insert("Bid", { lotId: args.lotId, amount: args.amount });
 *     return { accepted: true };
 *   },
 * });
 * ```
 */
export function mutation<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: MutationDef<TArgs, TReturn>
): FnDefinition<TArgs, TReturn> {
  return { type: "mutation", args: def.args, handler: def.handler };
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
 */
export function action<TArgs = Record<string, unknown>, TReturn = unknown>(
  def: ActionDef<TArgs, TReturn>
): FnDefinition<TArgs, TReturn> {
  return { type: "action", args: def.args, handler: def.handler };
}
