import { mutation, v } from "@pylonsync/functions";

// upvote — bump a listing's vote count. Public (anyone browsing can vote) and
// the realtime heart of the template: the Listing.votes write syncs to every
// open browse grid, so the count ticks up for everyone live. A per-listing
// advisory lock makes the read-then-increment race-safe under concurrent votes.
//
// Listings are deny-write to clients, so votes can ONLY move through here — a
// client can't forge a count. (Dedupe is best-effort on the client via
// localStorage; a production directory would tie votes to a session/Vote row.)
export default mutation<{ listingId: string }, { ok: boolean; votes: number }>({
  auth: "public",
  args: { listingId: v.id("Listing") },
  async handler(ctx, args) {
    await ctx.db.advisoryLock(`directory_listing:${args.listingId}`);
    const listing = (await ctx.db.get("Listing", args.listingId)) as
      | { votes: number }
      | null;
    if (!listing) throw ctx.error("NOT_FOUND", "Listing not found.");
    const votes = (listing.votes ?? 0) + 1;
    await ctx.db.unsafe.update("Listing", args.listingId, { votes });
    return { ok: true, votes };
  },
});
