import { mutation, v } from "@pylonsync/functions";

/**
 * Idempotent seed for first-launch demos. Drops in a couple of sample
 * auctions (one timed, one upcoming live) so a visitor sees a populated
 * homepage without having to create their own auction first.
 */
const TIMED_LOTS = [
  { title: "Vintage Leica M3", description: "1956 chrome body, working meter, near-mint condition.", startingCents: 80000 },
  { title: "Hand-bound first edition", description: "Lord of the Rings, 1954 Allen & Unwin first impression. Light foxing.", startingCents: 120000 },
  { title: "Mid-century Eames lounge", description: "Original Herman Miller production, rosewood/black leather. Reupholstered cushions.", startingCents: 240000 },
  { title: "1972 Patek Philippe Calatrava", description: "18k yellow gold, manual wind, original buckle.", startingCents: 480000 },
  { title: "Art Deco bronze sculpture", description: "Demêtre Chiparus, signed in the bronze. ~52 cm tall.", startingCents: 90000 },
  { title: "Le Corbusier LC2 sofa", description: "Cassina production, polished chrome frame, white leather.", startingCents: 220000 },
  { title: "Persian Tabriz silk rug", description: "Late 19th c., 3.2x2.1m, intricate medallion field.", startingCents: 75000 },
  { title: "Pair of Tiffany table lamps", description: "Stained glass shade with dragonfly motif. Both signed at the base.", startingCents: 180000 },
];

const LIVE_LOTS = [
  { title: "Original Banksy print, signed", description: "Girl with Balloon (2004), edition of 150. COA from POW.", startingCents: 1500000 },
  { title: "Stradivarius-school violin", description: "Cremonese, attributed to Carlo Bergonzi follower. Authenticated.", startingCents: 2200000 },
  { title: "Apollo 11 mission patch (flown)", description: "Flight-flown beta-cloth patch with Aldrin signature.", startingCents: 350000 },
  { title: "1957 Gibson Les Paul Goldtop", description: "All original P-90s, near-perfect finish. Provenance from a Nashville studio.", startingCents: 850000 },
  { title: "Boucheron diamond brooch", description: "5.4 ct center stone, art deco platinum mount. Restoration receipts included.", startingCents: 920000 },
  { title: "Ming dynasty porcelain vase", description: "Blue-and-white, Wanli period. Christie's provenance, 1987.", startingCents: 1100000 },
];

export default mutation({
  args: {
    force: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const existing = await ctx.db.query("Auction");
    if (existing.length > 0 && !args.force) {
      return { seeded: 0, existing: existing.length };
    }

    const now = new Date();
    let seeded = 0;

    // 1. Timed auction starting now, ending in 5 minutes.
    {
      const startsAt = now.toISOString();
      const endsAt = new Date(now.getTime() + 5 * 60 * 1000).toISOString();
      const auctionId = await ctx.db.insert("Auction", {
        title: "Spring Decorative Arts",
        description:
          "Eight curated lots — timed auction, each lot closes on its own clock.",
        kind: "timed",
        status: "running",
        creatorId: ctx.auth.userId,
        startsAt,
        endsAt,
        bannerColor: "#7c3aed",
        createdAt: now.toISOString(),
      });
      const perLotMs = (5 * 60 * 1000) / TIMED_LOTS.length;
      for (let i = 0; i < TIMED_LOTS.length; i++) {
        const lot = TIMED_LOTS[i];
        const lotEndsAt = new Date(
          now.getTime() + perLotMs * (i + 1),
        ).toISOString();
        await ctx.db.insert("Lot", {
          auctionId,
          position: i,
          title: lot.title,
          description: lot.description,
          imageColor: hashColor(lot.title),
          startingCents: lot.startingCents,
          currentCents: lot.startingCents,
          minIncrementCents: Math.max(100, Math.floor(lot.startingCents * 0.05)),
          bidCount: 0,
          status: "running",
          endsAt: lotEndsAt,
          createdAt: now.toISOString(),
        });
      }
      await ctx.scheduler.runAfter(2000, "sweepTimedLots", { auctionId });
      seeded++;
    }

    // 2. Live auction starting in 30 seconds.
    {
      const startsAt = new Date(now.getTime() + 30_000).toISOString();
      const endsAt = new Date(now.getTime() + 30 * 60_000).toISOString();
      const auctionId = await ctx.db.insert("Auction", {
        title: "Important Singles · Live",
        description:
          "Auctioneer-led live sale — open lots one at a time, antishill timer resets on every bid.",
        kind: "live",
        status: "scheduled",
        creatorId: ctx.auth.userId,
        startsAt,
        endsAt,
        bannerColor: "#ec4899",
        createdAt: now.toISOString(),
      });
      for (let i = 0; i < LIVE_LOTS.length; i++) {
        const lot = LIVE_LOTS[i];
        await ctx.db.insert("Lot", {
          auctionId,
          position: i,
          title: lot.title,
          description: lot.description,
          imageColor: hashColor(lot.title),
          startingCents: lot.startingCents,
          currentCents: lot.startingCents,
          minIncrementCents: Math.max(1000, Math.floor(lot.startingCents * 0.03)),
          bidCount: 0,
          status: "pending",
          endsAt: null,
          createdAt: now.toISOString(),
        });
      }
      await ctx.scheduler.runAfter(30_000, "startAuction", { auctionId });
      seeded++;
    }

    return { seeded };
  },
});

function hashColor(s: string): string {
  const palette = [
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
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return palette[Math.abs(h) % palette.length];
}
