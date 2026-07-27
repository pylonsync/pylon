import { mutation, v } from "@pylonsync/functions";
import { isValidStatus } from "../lib/billing";

/**
 * Move an invoice between draft, sent, paid, and void.
 *
 * "overdue" is deliberately NOT settable — it's derived from the due date and
 * the balance (lib/billing.ts), so there is no column to fall out of date and
 * no nightly job to run.
 *
 * Sending stamps the issue date if it's missing, because "sent" with no issue
 * date is a document that can't be aged.
 */
export default mutation<{ invoiceId: string; status: string }, { ok: true }>({
  auth: "user",
  args: { invoiceId: v.id("Invoice"), status: v.string() },
  async handler(ctx, args) {
    if (!isValidStatus(args.status)) {
      throw ctx.error("INVALID_ARGS", `Unknown status "${args.status}".`);
    }

    const invoice = await ctx.db.get("Invoice", args.invoiceId);
    if (!invoice) throw ctx.error("NOT_FOUND", "Invoice not found.");

    const now = new Date().toISOString();
    const patch: Record<string, unknown> = {
      status: args.status,
      updatedAt: now,
    };
    if (args.status === "sent" && !invoice.issueDate) patch.issueDate = now;

    await ctx.db.update("Invoice", args.invoiceId, patch);
    return { ok: true } as const;
  },
});
