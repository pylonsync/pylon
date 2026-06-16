import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// fulfillOrder — owner-only. Marks a reserved order fulfilled (shipped). Stock
// already came off the shelf at order time, so nothing else changes.
export default mutation<{ orderId: string }, { ok: boolean }>({
  auth: "user",
  args: { orderId: v.id("Order") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage orders.");
    }
    await ctx.db.unsafe.update("Order", args.orderId, { status: "fulfilled" });
    return { ok: true };
  },
});
