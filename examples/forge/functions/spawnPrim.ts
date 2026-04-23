import { mutation, v } from "@pylonsync/functions";

const KINDS = ["box", "sphere", "cone", "torus"];

export default mutation({
  args: {
    roomId: v.string(),
    kind: v.string(),
    x: v.optional(v.number()),
    y: v.optional(v.number()),
    z: v.optional(v.number()),
    color: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!KINDS.includes(args.kind)) {
      throw ctx.error("INVALID_KIND", `kind must be one of ${KINDS.join(", ")}`);
    }
    const id = await ctx.db.insert("Prim", {
      roomId: args.roomId,
      kind: args.kind,
      x: args.x ?? 0,
      y: args.y ?? 0.5,
      z: args.z ?? 0,
      sx: 1, sy: 1, sz: 1,
      color: args.color ?? "#8b5cf6",
      createdBy: ctx.auth.userId,
      updatedAt: new Date().toISOString(),
    });
    return { id };
  },
});
