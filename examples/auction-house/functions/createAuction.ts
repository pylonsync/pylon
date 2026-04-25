import { mutation, v } from "@pylonsync/functions";

/**
 * Create an Auction with N seeded lots in one transaction. The
 * auction starts in "scheduled" state and won't accept bids until
 * `startAuction` flips it to "running" (auctioneer-initiated for live,
 * scheduled for timed).
 */
const PALETTE = [
  "#8b5cf6",
  "#6366f1",
  "#3b82f6",
  "#06b6d4",
  "#10b981",
  "#84cc16",
  "#eab308",
  "#f97316",
  "#ef4444",
  "#ec4899",
];

function pickColor(seed: string) {
  let h = 0;
  for (const c of seed) h = (h * 31 + c.charCodeAt(0)) | 0;
  return PALETTE[Math.abs(h) % PALETTE.length];
}

export default mutation({
  args: {
    title: v.string(),
    description: v.string(),
    kind: v.string(),
    startsAt: v.string(),
    durationSecs: v.int(),
    lots: v.array(
      v.object({
        title: v.string(),
        description: v.string(),
        startingCents: v.int(),
      }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (args.lots.length === 0)
      throw ctx.error("EMPTY_AUCTION", "add at least one lot");
    if (args.kind !== "timed" && args.kind !== "live") {
      throw ctx.error("BAD_KIND", "kind must be 'timed' or 'live'");
    }
    const now = new Date();
    const starts = new Date(args.startsAt);
    if (Number.isNaN(starts.getTime())) {
      throw ctx.error("BAD_START", "startsAt must be a valid ISO date");
    }
    const ends = new Date(starts.getTime() + args.durationSecs * 1000);

    const auctionId = await ctx.db.insert("Auction", {
      title: args.title,
      description: args.description,
      kind: args.kind,
      status: "scheduled",
      creatorId: ctx.auth.userId,
      startsAt: starts.toISOString(),
      endsAt: ends.toISOString(),
      bannerColor: pickColor(args.title),
      createdAt: now.toISOString(),
    });

    // For timed auctions, distribute lot deadlines evenly across the
    // auction window so they don't all close at the same instant.
    const perLotMs =
      args.kind === "timed"
        ? Math.floor((ends.getTime() - starts.getTime()) / args.lots.length)
        : 0;

    for (let i = 0; i < args.lots.length; i++) {
      const lot = args.lots[i];
      const lotEndsAt =
        args.kind === "timed"
          ? new Date(starts.getTime() + perLotMs * (i + 1)).toISOString()
          : null;

      await ctx.db.insert("Lot", {
        auctionId,
        position: i,
        title: lot.title,
        description: lot.description,
        imageColor: pickColor(`${args.title}::${lot.title}::${i}`),
        startingCents: lot.startingCents,
        currentCents: lot.startingCents,
        minIncrementCents: Math.max(100, Math.floor(lot.startingCents * 0.05)),
        bidCount: 0,
        status: "pending",
        endsAt: lotEndsAt,
        createdAt: now.toISOString(),
      });
    }

    // Schedule the auction's lifecycle. For a timed auction, every lot
    // closes on its own deadline — kick off a single sweeper that runs
    // periodically while the auction is active. For a live auction,
    // we don't schedule anything: the auctioneer drives lot transitions
    // manually.
    const startDelayMs = Math.max(0, starts.getTime() - now.getTime());
    await ctx.scheduler.runAfter(startDelayMs, "startAuction", { auctionId });
    if (args.kind === "timed") {
      // First sweep at start time + 1s; the function reschedules itself
      // until all lots are closed.
      await ctx.scheduler.runAfter(startDelayMs + 1000, "sweepTimedLots", {
        auctionId,
      });
    }

    return { auctionId, lotCount: args.lots.length };
  },
});
