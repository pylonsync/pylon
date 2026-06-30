import { mutation } from "@pylonsync/functions";

// A few demo posts so the feed isn't an empty shell on first visit. The feed
// calls this on mount; it's a no-op once any post exists (the lock guards
// against a double-seed from two concurrent first-visits). Public so an
// anonymous first visitor seeds it — it only writes demo content.
//
// `unsafe.insert` sets the demo `authorId` + a backdated `createdAt` directly
// (bypassing the `field.owner()` stamp + policies) so the seed reads like a few
// different people already posted, instead of everything attributed to the
// first visitor.
const DEMO_POSTS: { author: string; text: string }[] = [
  {
    author: "guest_ada",
    text: "just shipped my first Pylon app — one binary doing SSR + sync + auth. wild.",
  },
  {
    author: "guest_lin",
    text: "the feed updates across tabs with zero websocket code I wrote. open a second tab 👀",
  },
  {
    author: "guest_rey",
    text: "likes are just rows — delete the row to unlike. local-first, so it's instant.",
  },
  {
    author: "guest_max",
    text: "no separate backend to deploy. `pylon deploy` and it's live. that's the whole thing.",
  },
];

export default mutation<
  Record<string, never>,
  { seeded: boolean; count: number }
>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("consumer_seed_posts");
    const existing = await ctx.db.unsafe.list("Post");
    if (existing.length > 0) return { seeded: false, count: existing.length };

    const now = Date.now();
    for (let i = 0; i < DEMO_POSTS.length; i++) {
      const p = DEMO_POSTS[i];
      // Backdate a minute apart so they sort into a natural order.
      const createdAt = new Date(
        now - (DEMO_POSTS.length - i) * 60_000,
      ).toISOString();
      await ctx.db.unsafe.insert("Post", {
        authorId: p.author,
        text: p.text,
        createdAt,
      });
    }
    return { seeded: true, count: DEMO_POSTS.length };
  },
});
