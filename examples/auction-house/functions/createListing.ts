import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    title: v.string(),
    description: v.string(),
    startingCents: v.number(),
    durationSecs: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw new Error("auth required");
    if (args.startingCents < 1) throw new Error("starting price must be > 0");
    if (args.durationSecs < 60) throw new Error("auction must run at least 60s");

    const now = new Date().toISOString();
    const endsAtMs = Date.now() + args.durationSecs * 1000;
    const endsAt = new Date(endsAtMs).toISOString();

    const id = await ctx.db.insert("Listing", {
      title: args.title,
      description: args.description,
      sellerId: ctx.auth.userId,
      startingCents: args.startingCents,
      currentCents: args.startingCents,
      winningBidId: null,
      endsAt,
      settledAt: null,
      createdAt: now,
    });

    await ctx.scheduler.runAt(endsAtMs, "settleListing", { listingId: id });

    return { id, endsAt };
  },
});
