import { mutation, v } from "@pylonsync/functions";

const KINDS = ["box", "sphere", "cone", "torus"];

export default mutation({
  auth: "guest",
  args: {
    roomId: v.string(),
    kind: v.string(),
    x: v.optional(v.number()),
    y: v.optional(v.number()),
    z: v.optional(v.number()),
    color: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const a = args as {
      roomId: string;
      kind: string;
      x?: number;
      y?: number;
      z?: number;
      color?: string;
    };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!KINDS.includes(a.kind)) {
      throw ctx.error("INVALID_KIND", `kind must be one of ${KINDS.join(", ")}`);
    }
    const id = await ctx.db.insert("Prim", {
      roomId: a.roomId,
      kind: a.kind,
      x: a.x ?? 0,
      y: a.y ?? 0.5,
      z: a.z ?? 0,
      sx: 1, sy: 1, sz: 1,
      color: a.color ?? "#8b5cf6",
      createdBy: ctx.auth.userId,
      updatedAt: new Date().toISOString(),
    });
    return { id };
  },
});
