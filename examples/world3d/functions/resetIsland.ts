import { mutation } from "@pylonsync/functions";

/**
 * Restore every destroyed building by deleting all Destruction rows.
 * The live query fans the deletes out and each client rebuilds its
 * block instances. Any signed-in player can reset — it's a demo world.
 */
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const rows = await ctx.db.query("Destruction", {});
    for (const row of rows) await ctx.db.delete("Destruction", row.id as string);
    return { deleted: rows.length };
  },
});
