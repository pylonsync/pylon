import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { InvoiceRow } from "../lib/agency";

// deleteClient — owner-only. Refuses to delete a client that still has invoices
// (a financial record must not lose its counterparty); the owner deletes or
// reassigns those invoices first. Returns `{ ok: false, invoices: n }` so the
// dashboard can explain why.
export default mutation<{ id: string }, { ok: boolean; invoices: number }>({
  auth: "user",
  args: { id: v.id("Client") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage clients.");
    }
    const client = await ctx.db.get("Client", args.id);
    if (!client) return { ok: true, invoices: 0 };

    const invoices = (await ctx.db.unsafe.list("Invoice")) as unknown as InvoiceRow[];
    const linked = invoices.filter((i) => i.clientId === args.id).length;
    if (linked > 0) return { ok: false, invoices: linked };

    await ctx.db.unsafe.delete("Client", args.id);
    return { ok: true, invoices: 0 };
  },
});
