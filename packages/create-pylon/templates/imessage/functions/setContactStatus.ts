import { mutation, v } from "@pylonsync/functions";

// Allow, block, or reset a contact. Allowing a contact who has texts waiting
// does not answer them; use answerNow for that. Owner only.
export default mutation({
  auth: "admin",
  args: {
    contactId: v.string(),
    status: v.union(v.literal("allowed"), v.literal("blocked"), v.literal("unknown")),
  },
  async handler(ctx, args) {
    const contact = await ctx.db.get("Contact", args.contactId);
    if (!contact) throw ctx.error("NOT_FOUND", "No such contact");
    await ctx.db.update("Contact", args.contactId, { status: args.status });
    return { ok: true };
  },
});
