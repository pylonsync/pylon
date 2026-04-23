import { mutation, v } from "@pylonsync/functions";

/**
 * Add a panel to the org dashboard. `spec` should be a JSON-encoded
 * AggregateSpec — validated shape-wise but not executed here; the
 * /api/aggregate endpoint is what actually evaluates it, so the usual
 * column-allowlist + policy gates apply at query time.
 */
export default mutation({
  args: {
    title: v.string(),
    entity: v.string(),
    chartKind: v.string(), // "number" | "bar" | "line"
    specJson: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    if (args.title.trim().length === 0) {
      throw ctx.error("INVALID_TITLE", "title is required");
    }
    const validKinds = ["number", "bar", "line"];
    if (!validKinds.includes(args.chartKind)) {
      throw ctx.error("INVALID_KIND", `chartKind ∈ ${validKinds.join(", ")}`);
    }

    // Shape-check the spec — we don't execute it, but bad JSON should
    // fail at create time rather than on first render.
    try {
      const parsed = JSON.parse(args.specJson);
      if (parsed === null || typeof parsed !== "object") {
        throw new Error("spec must be an object");
      }
    } catch (e) {
      throw ctx.error(
        "INVALID_SPEC",
        e instanceof Error ? e.message : "invalid spec JSON",
      );
    }

    // Append to the end of the current panel list.
    const existing = await ctx.db.query("DashboardPanel", {
      orgId: ctx.auth.tenantId,
    });
    const sortOrder = existing.reduce(
      (max: number, p: { sortOrder?: number }) =>
        Math.max(max, p.sortOrder ?? 0),
      0,
    ) + 1;

    const id = await ctx.db.insert("DashboardPanel", {
      orgId: ctx.auth.tenantId,
      title: args.title.trim(),
      entity: args.entity,
      chartKind: args.chartKind,
      specJson: args.specJson,
      sortOrder,
      createdBy: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });
    return { panelId: id };
  },
});
