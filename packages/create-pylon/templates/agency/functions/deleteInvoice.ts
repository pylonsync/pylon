import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// deleteInvoice — owner-only. Removes a bill outright (e.g. a draft created in
// error). No cascade — invoices don't own anything.
export default mutation<{ id: string }, { ok: boolean }>({
  auth: "user",
  args: { id: v.id("Invoice") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage invoices.");
    }
    const invoice = await ctx.db.get("Invoice", args.id);
    if (!invoice) return { ok: true };
    await ctx.db.unsafe.delete("Invoice", args.id);
    return { ok: true };
  },
});
