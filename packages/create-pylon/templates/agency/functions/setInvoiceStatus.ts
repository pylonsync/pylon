import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// setInvoiceStatus — owner-only. Move a bill along its lifecycle
// (draft → sent → paid, or → overdue) without re-opening the full editor. The
// dashboard's per-invoice status menu calls this.
const STATUSES = new Set(["draft", "sent", "paid", "overdue"]);

export default mutation<{ id: string; status: string }, { ok: boolean }>({
  auth: "user",
  args: { id: v.id("Invoice"), status: v.string() },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage invoices.");
    }
    const status = args.status.trim();
    if (!STATUSES.has(status)) {
      throw ctx.error("INVALID_ARGS", "Status must be draft, sent, paid, or overdue.");
    }
    const invoice = await ctx.db.get("Invoice", args.id);
    if (!invoice) throw ctx.error("NOT_FOUND", "Invoice not found.");

    await ctx.db.unsafe.update("Invoice", args.id, { status });
    return { ok: true };
  },
});
