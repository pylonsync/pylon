import { mutation, v } from "@pylonsync/functions";

/**
 * Record destroyed building blocks. Buildings are generated
 * deterministically from the world seed, so a block's coordinate key
 * ("<building>:<gx>:<gy>:<gz>") identifies the same block on every
 * client. Structural collapse is a pure function of the destroyed-key
 * set, so only directly-hit blocks need to be recorded — every client
 * derives the identical rubble.
 *
 * Inserts are idempotent: keys that already exist are skipped, so two
 * players shooting the same block don't race into a unique-index error.
 */
export default mutation({
  auth: "guest",
  args: {
    keys: v.array(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const keys = args.keys as string[];
    if (keys.length > 64) throw ctx.error("INVALID_ARGS", "too many keys in one call");

    const now = new Date().toISOString();
    let inserted = 0;
    for (const key of keys) {
      if (typeof key !== "string" || key.length === 0 || key.length > 64) continue;
      const existing = await ctx.db.query("Destruction", { key });
      if (existing.length > 0) continue;
      await ctx.db.insert("Destruction", {
        key,
        userId: ctx.auth.userId,
        createdAt: now,
      });
      inserted++;
    }
    return { inserted };
  },
});
