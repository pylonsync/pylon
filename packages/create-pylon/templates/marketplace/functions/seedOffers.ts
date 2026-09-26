import { mutation } from "@pylonsync/functions";

// Demo offers for first boot, made by the caller (the "collector" account in
// client/market.ts). Each entry is an ordinary offer the caller could send
// through makeOffer, or a purchase it could make through buyNow, so this
// grants no extra power: it skips listings the caller owns and listings that
// are no longer active. Idempotent per caller: a buyer that already has
// offers is skipped.
const PLAN: Array<{
  seed: string;
  amount?: number;
  buyerName: string;
  message?: string;
  minutesAgo: number;
  buy?: boolean;
}> = [
  { seed: "a1f3", amount: 600, buyerName: "Leo T.", minutesAgo: 31 },
  { seed: "a1f3", amount: 620, buyerName: "Ines M.", message: "Could pick up this weekend.", minutesAgo: 6 },
  { seed: "b7c2", amount: 800, buyerName: "Kenji A.", minutesAgo: 14 },
  { seed: "c4e9", amount: 215, buyerName: "Dana R.", message: "Does the meter need a battery?", minutesAgo: 26 },
  { seed: "d8a1", amount: 380, buyerName: "Ines M.", minutesAgo: 22 },
  { seed: "a9c4", amount: 260, buyerName: "Leo T.", minutesAgo: 44 },
  { seed: "e2b6", buyerName: "Kenji A.", minutesAgo: 11, buy: true },
  { seed: "n8w3", amount: 400, buyerName: "Dana R.", message: "I can pick up Saturday morning.", minutesAgo: 3 },
  { seed: "p2d6", amount: 60, buyerName: "Ines M.", minutesAgo: 17 },
];

interface SeedOffersResult {
  seeded: number;
}

type Row = {
  id: string;
  seed: string;
  title: string;
  price: number;
  sellerId: string;
  status: string;
};

export default mutation<Record<string, never>, SeedOffersResult>({
  args: {},
  async handler(ctx) {
    const buyerId = ctx.auth.userId;
    if (!buyerId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    const existing = (await ctx.db.list("Offer")) as Array<{ buyerId: string }>;
    if (existing.some((o) => o.buyerId === buyerId)) return { seeded: 0 };

    const listings = (await ctx.db.list("Listing")) as Row[];
    const bySeed = new Map(listings.map((l) => [l.seed, l]));
    const now = Date.now();
    let seeded = 0;

    for (const entry of PLAN) {
      const listing = bySeed.get(entry.seed);
      if (!listing || listing.sellerId === buyerId || listing.status !== "active") {
        continue;
      }
      const createdAt = new Date(now - entry.minutesAgo * 60_000).toISOString();

      if (entry.buy) {
        const offerId = await ctx.db.insert("Offer", {
          listingId: listing.id,
          listingTitle: listing.title,
          sellerId: listing.sellerId,
          buyerId,
          buyerName: entry.buyerName,
          amount: listing.price,
          message: "Bought at list price",
          status: "accepted",
          createdAt,
        });
        await ctx.db.update("Listing", listing.id, { status: "sold" });
        listing.status = "sold";
        const siblings = (await ctx.db.query("Offer", {
          listingId: listing.id,
        })) as Array<{ id: string; status: string }>;
        for (const o of siblings) {
          if (o.id !== offerId && o.status === "pending") {
            await ctx.db.update("Offer", o.id, { status: "declined" });
          }
        }
      } else {
        await ctx.db.insert("Offer", {
          listingId: listing.id,
          listingTitle: listing.title,
          sellerId: listing.sellerId,
          buyerId,
          buyerName: entry.buyerName,
          amount: entry.amount ?? listing.price,
          message: entry.message ?? "",
          status: "pending",
          createdAt,
        });
      }
      seeded++;
    }
    return { seeded };
  },
});
