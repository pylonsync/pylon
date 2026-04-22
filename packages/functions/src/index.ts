/**
 * @statecraft/functions — TypeScript function definitions for statecraft.
 *
 * This is the developer-facing API. App developers import from here
 * to define queries, mutations, and actions.
 *
 * @example
 * ```typescript
 * import { mutation, v } from "@statecraft/functions";
 *
 * export default mutation({
 *   args: { lotId: v.string(), amount: v.number() },
 *   async handler(ctx, args) {
 *     const lot = await ctx.db.get("Lot", args.lotId);
 *     // ...
 *   },
 * });
 * ```
 */

export { query, mutation, action } from "./define";
export { v } from "./validators";
export { resetDb, installTestIsolation } from "./testing";
export type {
  QueryCtx,
  MutationCtx,
  ActionCtx,
  DbReader,
  DbWriter,
  Stream,
  Scheduler,
  AuthInfo,
  FnDefinition,
} from "./types";
