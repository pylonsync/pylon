import { mutation, v } from "@pylonsync/functions";
import { totals, type LineItem, type Payment } from "../lib/billing";

/**
 * Record money received against an invoice.
 *
 * A mutation rather than a bare insert because a payment can settle the
 * invoice, and the status flip has to happen in the same transaction — a
 * payment that lands without the status changing leaves someone chasing an
 * invoice that's already been paid.
 *
 * The balance is recomputed SERVER-side from the line items and existing
 * payments. Trusting a client-sent balance would let a stale tab overpay or
 * mark an invoice settled that isn't.
 */
export default mutation<
  { invoiceId: string; amountCents: number; method?: string; reference?: string },
  { ok: true; settled: boolean }
>({
  auth: "user",
  args: {
    invoiceId: v.id("Invoice"),
    amountCents: v.number(),
    method: v.optional(v.string()),
    reference: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const amountCents = Math.round(Number(args.amountCents) || 0);
    if (amountCents <= 0) {
      throw ctx.error("INVALID_ARGS", "A payment must be a positive amount.");
    }

    const invoice = await ctx.db.get("Invoice", args.invoiceId);
    if (!invoice) throw ctx.error("NOT_FOUND", "Invoice not found.");
    if (invoice.status === "void") {
      throw ctx.error("INVALID_ARGS", "That invoice is void.");
    }

    const items = (await ctx.db.query("LineItem", {
      invoiceId: args.invoiceId,
    })) as unknown as LineItem[];
    const payments = (await ctx.db.query("Payment", {
      invoiceId: args.invoiceId,
    })) as unknown as Payment[];

    const before = totals(invoice as never, items, payments);
    if (amountCents > before.balanceCents) {
      throw ctx.error(
        "INVALID_ARGS",
        "That is more than the outstanding balance.",
      );
    }

    const now = new Date().toISOString();
    await ctx.db.insert("Payment", {
      invoiceId: args.invoiceId,
      amountCents,
      method: args.method ?? "other",
      reference: args.reference || null,
      paidAt: now,
      recordedBy: ctx.auth.userId,
    });

    const settled = amountCents >= before.balanceCents;
    await ctx.db.update("Invoice", args.invoiceId, {
      updatedAt: now,
      ...(settled ? { status: "paid" } : {}),
    });

    return { ok: true, settled } as const;
  },
});
