import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { InvoiceRow, OwnerInvoicesResult } from "../lib/agency";

// invoicesForOwner — the owner's billing. Invoices hold money + client data, so
// the entity denies all client access; this owner-gated query is the only way
// to read them. Like the other owner queries it returns `{ authorized: false }`
// for a non-owner (a query has no `ctx.error`) rather than throwing. Sorted
// newest-first by issue date, falling back to creation time.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerInvoicesResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Invoice")) as unknown as InvoiceRow[];
    const key = (i: InvoiceRow) => i.issuedAt || i.createdAt;
    const invoices = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (key(a) < key(b) ? 1 : key(a) > key(b) ? -1 : 0));

    return { authorized: true, invoices };
  },
});
