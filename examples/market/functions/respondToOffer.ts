import { mutation, v } from "@pylonsync/functions";

/**
 * Seller accepts or declines an offer. Accepting is the interesting path:
 * it marks the offer accepted, flips the listing to "sold", and declines
 * every other pending offer on that listing — all in one mutation, fanned
 * out to every watching tab by the change log.
 *
 * Ownership is enforced here (not in a policy expression) because the rule
 * spans two entities: "the caller must own the LISTING the OFFER points at."
 */
export default mutation({
  auth: "guest",
  args: {
    offerId: v.id("Offer"),
    accept: v.boolean(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    const offer = await ctx.db.get("Offer", args.offerId);
    if (!offer) throw ctx.error("NOT_FOUND", "offer not found");
    if (offer.sellerId !== ctx.auth.userId)
      throw ctx.error("UNAUTHORIZED", "only the seller can answer this offer");
    if (offer.status !== "pending")
      throw ctx.error("INVALID_ARGS", "this offer was already answered");

    if (!args.accept) {
      await ctx.db.update("Offer", args.offerId, { status: "declined" });
      return { accepted: false };
    }

    await ctx.db.update("Offer", args.offerId, { status: "accepted" });
    await ctx.db.update("Listing", String(offer.listingId), { status: "sold" });

    // Decline the losers.
    const siblings = await ctx.db.query("Offer", {
      listingId: offer.listingId,
    });
    for (const o of siblings) {
      if (o.id !== args.offerId && o.status === "pending") {
        await ctx.db.update("Offer", String(o.id), { status: "declined" });
      }
    }
    return { accepted: true };
  },
});
