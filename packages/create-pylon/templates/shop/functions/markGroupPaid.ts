import { mutation, v } from "@pylonsync/functions";

// markGroupPaid — flip a cart's order lines from "pending" to "paid" once Stripe
// confirms the payment. Called only by the stripeWebhook action (internal), and
// idempotent: it only touches rows still "pending", so a re-delivered webhook
// (Stripe retries) is a no-op. Stock was already held at checkout, so nothing
// else changes here.
export default mutation<{ orderGroupId: string }, { ok: boolean; updated: number }>({
  internal: true,
  args: { orderGroupId: v.string() },
  async handler(ctx, args) {
    const rows = (await ctx.db.unsafe.list("Order")) as unknown as {
      id: string;
      orderGroupId: string;
      status: string;
    }[];
    let updated = 0;
    for (const o of rows) {
      if (o.orderGroupId === args.orderGroupId && o.status === "pending") {
        await ctx.db.unsafe.update("Order", o.id, { status: "paid" });
        updated++;
      }
    }
    return { ok: true, updated };
  },
});
