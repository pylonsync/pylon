import { mutation, v } from "@pylonsync/functions";

/**
 * The smallest useful mutation: upsert a Counter row + increment it.
 * Workers call this at a configurable rate; every call is a full
 * round-trip that gets timed client-side.
 */
export default mutation({
  args: {
    label: v.string(),
    delta: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const rows = await ctx.db.query("Counter", { label: args.label });
    const now = new Date().toISOString();
    if (rows.length > 0) {
      const row = rows[0];
      await ctx.db.update("Counter", row.id as string, {
        value: (row.value as number) + args.delta,
        updatedAt: now,
      });
      return { value: (row.value as number) + args.delta };
    }
    await ctx.db.insert("Counter", {
      label: args.label,
      value: args.delta,
      updatedAt: now,
    });
    return { value: args.delta };
  },
});
