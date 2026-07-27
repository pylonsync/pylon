import { mutation, v } from "@pylonsync/functions";
import { onHand, reasonAllows, type Movement } from "../lib/stock";

/**
 * Append a stock movement.
 *
 * This is the ONLY way stock changes. There is no quantity column to update —
 * on-hand is the sum of these rows — so two people receiving the same delivery
 * produce two rows and the correct total, instead of racing on a counter and
 * losing units.
 *
 * The reason has to agree with the direction: a "Sold" that ADDS stock is a
 * mis-click, and letting it through makes the ledger untrustworthy, which is
 * the only thing the ledger has going for it.
 *
 * Issuing more than you hold is refused. Negative stock is always either a
 * miscount or a mistake, and silently recording it turns one error into a
 * valuation that quietly lies. Correct with a "Stock count" movement instead.
 */
export default mutation<
  { productId: string; delta: number; reason: string; note?: string },
  { ok: true; onHand: number }
>({
  auth: "user",
  args: {
    productId: v.id("Product"),
    delta: v.number(),
    reason: v.string(),
    note: v.optional(v.string()),
  },
  async handler(ctx, args) {
    // Whole units only — half a physical thing is a unit-of-measure mistake.
    const delta = Math.trunc(Number(args.delta) || 0);
    if (delta === 0) {
      throw ctx.error("INVALID_ARGS", "A movement has to change the count.");
    }
    if (!reasonAllows(args.reason, delta)) {
      throw ctx.error(
        "INVALID_ARGS",
        `"${args.reason}" can't move stock in that direction.`,
      );
    }

    const product = await ctx.db.get("Product", args.productId);
    if (!product) throw ctx.error("NOT_FOUND", "Product not found.");

    const movements = (await ctx.db.query("Movement", {
      productId: args.productId,
    })) as unknown as Movement[];
    const current = onHand(args.productId, movements);

    // A "count" movement is the correction mechanism, so it's allowed to go
    // wherever the shelf actually is.
    if (args.reason !== "count" && current + delta < 0) {
      throw ctx.error(
        "INSUFFICIENT_STOCK",
        `Only ${current} in stock — record a stock count if the shelf disagrees.`,
      );
    }

    await ctx.db.insert("Movement", {
      productId: args.productId,
      delta,
      reason: args.reason,
      note: args.note?.trim() || null,
      actorId: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });

    return { ok: true, onHand: current + delta } as const;
  },
});
