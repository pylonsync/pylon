import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    blockId: v.string(),
    direction: v.string(), // "up" | "down"
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const block = await ctx.db.get("Block", args.blockId);
    if (!block || (block as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "block not found");

    const b = block as { pageId: string; parentId?: string; sort: number };
    const siblings = await ctx.db.query("Block", {
      pageId: b.pageId,
      parentId: b.parentId ?? null,
    });
    const sorted = [...siblings].sort(
      (a, c) => (a.sort as number) - (c.sort as number),
    );
    const idx = sorted.findIndex((s) => s.id === args.blockId);
    if (idx < 0) throw ctx.error("NOT_FOUND", "block not in page");

    // Fractional-sort swap — drop the block's sort between its new
    // neighbors so only one row is rewritten instead of the full list.
    let newSort: number;
    if (args.direction === "up") {
      if (idx === 0) return { ok: false, reason: "at top" };
      const prev = sorted[idx - 1].sort as number;
      const prevPrev = idx >= 2 ? (sorted[idx - 2].sort as number) : prev - 1;
      newSort = (prev + prevPrev) / 2;
    } else if (args.direction === "down") {
      if (idx === sorted.length - 1) return { ok: false, reason: "at bottom" };
      const next = sorted[idx + 1].sort as number;
      const nextNext = idx + 2 < sorted.length
        ? (sorted[idx + 2].sort as number)
        : next + 1;
      newSort = (next + nextNext) / 2;
    } else {
      throw ctx.error("INVALID_DIRECTION", "direction must be up or down");
    }

    await ctx.db.update("Block", args.blockId, { sort: newSort });
    return { ok: true };
  },
});
