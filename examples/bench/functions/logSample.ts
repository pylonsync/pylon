import { mutation, v } from "@pylonsync/functions";

/**
 * Record a 1-second sample during a run. The main tab uploads these
 * every second so the Sample log can be replayed / compared across
 * runs without relying on in-memory state.
 */
export default mutation({
  args: {
    runId: v.string(),
    atSec: v.number(),
    mutations: v.number(),
    p50ms: v.number(),
    p95ms: v.number(),
    p99ms: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.insert("Sample", {
      runId: args.runId,
      atSec: args.atSec,
      mutations: args.mutations,
      p50ms: args.p50ms,
      p95ms: args.p95ms,
      p99ms: args.p99ms,
    });
    return { ok: true };
  },
});
