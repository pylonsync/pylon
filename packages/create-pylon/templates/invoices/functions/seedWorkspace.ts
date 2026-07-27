import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Fill a brand-new billing workspace once.
 *
 * An empty invoice list shows nothing about totals, ageing, or partial payment.
 * The client calls this after sign-in; it returns immediately if any invoice
 * exists, so it's safe on every load and can never duplicate the fixtures or
 * touch real books.
 *
 * Delete this function and lib/seed.ts once you're billing real clients.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    const existing = await ctx.db.query("Invoice", { $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    const clientIds = new Map<string, string>();
    for (const client of seed.clients) {
      const id = await ctx.db.insert("Client", client.row);
      clientIds.set(client.key, id as string);
    }

    const invoiceIds = new Map<string, string>();
    for (const invoice of seed.invoices) {
      const id = await ctx.db.insert("Invoice", {
        ...invoice.row,
        clientId: clientIds.get(invoice.client) ?? null,
        ownerId: me,
      });
      invoiceIds.set(invoice.key, id as string);
    }

    for (const line of seed.lines) {
      await ctx.db.insert("LineItem", {
        ...line.row,
        invoiceId: invoiceIds.get(line.invoice) ?? null,
      });
    }

    for (const payment of seed.payments) {
      await ctx.db.insert("Payment", {
        ...payment.row,
        invoiceId: invoiceIds.get(payment.invoice) ?? null,
        recordedBy: me,
      });
    }

    return { seeded: true };
  },
});
