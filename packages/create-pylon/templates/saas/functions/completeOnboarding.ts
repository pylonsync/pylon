import { mutation, v } from "@pylonsync/functions";

// Marks the workspace's first-run wizard as finished. Org rows are
// framework-managed and deny client writes, so the stamp goes through this
// owner/admin-gated function.
export default mutation<{ orgId: string }, { ok: true }>({
  args: { orgId: v.string() },
  async handler(ctx, args) {
    await ctx.requireMember(args.orgId, { role: ["owner", "admin"] });
    await ctx.db.unsafe.update("Org", args.orgId, {
      onboardedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
