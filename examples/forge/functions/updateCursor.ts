import { mutation, v } from "@pylonsync/functions";

/**
 * Upsert the caller's cursor position in a room. Called at ~20 Hz
 * while the pointer is over the scene; other clients see it via
 * the Cursor live query.
 */
export default mutation({
  auth: "guest",
  args: {
    roomId: v.string(),
    name: v.string(),
    color: v.string(),
    x: v.number(),
    y: v.number(),
    z: v.number(),
  },
  async handler(ctx, args) {
    const a = args as {
      roomId: string;
      name: string;
      color: string;
      x: number;
      y: number;
      z: number;
    };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const rows = await ctx.db.query("Cursor", {
      roomId: a.roomId, userId: ctx.auth.userId,
    });
    const now = new Date().toISOString();
    if (rows.length > 0) {
      await ctx.db.update("Cursor", rows[0].id as string, {
        x: a.x, y: a.y, z: a.z,
        name: a.name, color: a.color,
        updatedAt: now,
      });
    } else {
      await ctx.db.insert("Cursor", {
        roomId: a.roomId,
        userId: ctx.auth.userId,
        name: a.name, color: a.color,
        x: a.x, y: a.y, z: a.z,
        updatedAt: now,
      });
    }
    return { ok: true };
  },
});
