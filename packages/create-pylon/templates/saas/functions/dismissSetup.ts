import { mutation, v } from "@pylonsync/functions";

// Hides the "Getting started" checklist on the Overview for everyone in the
// workspace. Owners and admins only, since it is a workspace-wide setting.
export default mutation<{ orgId: string }, { ok: true }>({
  args: { orgId: v.string() },
  async handler(ctx, args) {
    await ctx.requireMember(args.orgId, { role: ["owner", "admin"] });
    await ctx.db.unsafe.update("Org", args.orgId, {
      setupDismissedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
