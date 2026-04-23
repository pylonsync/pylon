import { mutation, v } from "@pylonsync/functions";

/**
 * Upsert the caller's cursor position in a room. Called at ~20 Hz
 * while the pointer is over the scene; other clients see it via
 * the Cursor live query.
 */
export default mutation({
  args: {
    roomId: v.string(),
    name: v.string(),
    color: v.string(),
    x: v.number(),
    y: v.number(),
    z: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const rows = await ctx.db.query("Cursor", {
      roomId: args.roomId, userId: ctx.auth.userId,
    });
    const now = new Date().toISOString();
    if (rows.length > 0) {
      await ctx.db.update("Cursor", rows[0].id as string, {
        x: args.x, y: args.y, z: args.z,
        name: args.name, color: args.color,
        updatedAt: now,
      });
    } else {
      await ctx.db.insert("Cursor", {
        roomId: args.roomId,
        userId: ctx.auth.userId,
        name: args.name, color: args.color,
        x: args.x, y: args.y, z: args.z,
        updatedAt: now,
      });
    }
    return { ok: true };
  },
});
