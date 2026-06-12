/**
 * @pylonsync/functions — TypeScript function definitions for pylon.
 *
 * This is the developer-facing API. App developers import from here
 * to define queries, mutations, and actions.
 *
 * @example
 * ```typescript
 * import { mutation, v } from "@pylonsync/functions";
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
export { slugifyName, availableSlug } from "./slugify";
// SSR page response controller — pages/layouts receive `response` in
// props (response.setStatus / redirect / notFound / setHeader / setCookie).
// Type-only; the runtime instance is injected per-render by the SSR adapter.
export type {
  SsrResponse,
  SsrCookieOptions,
  SsrMetadata,
  Sitemap,
  SitemapEntry,
  Robots,
  RobotsRule,
} from "./ssr-runtime";
export type {
  QueryCtx,
  MutationCtx,
  ActionCtx,
  DbReader,
  DbWriter,
  Stream,
  Scheduler,
  AuthInfo,
  AuthMode,
  AuthRequirement,
  FnDefinition,
} from "./types";
