import { mutation } from "@pylonsync/functions";

// Demo posts so the feed is not an empty shell on first visit. The feed
// calls this on mount; it's a no-op once any post exists (the lock guards
// against a double-seed from two concurrent first-visits). Public so an
// anonymous first visitor seeds it — it only writes demo content.
//
// `unsafe.insert` sets the demo `authorId` + a backdated `createdAt` directly
// (bypassing the `field.owner()` stamp + policies) so the seed reads like a few
// different people already posted, instead of everything attributed to the
// first visitor.
const DEMO_POSTS: { author: string; text: string }[] = [
  { author: "guest_ada", text: "just shipped my first Pylon app. one binary doing SSR, sync, and auth." },
  { author: "guest_lin", text: "the feed updates across tabs with zero websocket code I wrote. open a second tab and watch." },
  { author: "guest_rey", text: "likes are just rows. delete the row to unlike. local-first, so it is instant." },
  { author: "guest_max", text: "no separate backend to deploy. pylon deploy and it is live. that is the whole thing." },
  { author: "guest_noor", text: "moved a side project over this weekend. the schema file replaced about 900 lines of API glue." },
  { author: "guest_theo", text: "policies in the manifest instead of middleware. took me a day to trust it, now I would not go back." },
  { author: "guest_ada", text: "follow-up: added comments this morning. one entity, one policy, one component." },
  { author: "guest_ivy", text: "anyone else running this on Fly? curious what the cold start looks like at the edge." },
  { author: "guest_lin", text: "@guest_ivy about 300ms for me from Sydney. the sync socket reconnects on its own after a deploy." },
  { author: "guest_sam", text: "the thing I did not expect: the optimistic writes make the app feel native on a bad connection." },
  { author: "guest_rey", text: "wrote up how the offline queue works. link in bio. short version: it is a log, not a cache." },
  { author: "guest_max", text: "hot take: most apps do not need a separate API. they need a schema and a sync layer." },
  { author: "guest_noor", text: "seeded 40 rows of demo data instead of 4 and the screenshots stopped looking like a toy." },
  { author: "guest_theo", text: "shipping v2 tonight. the migration is one field added to the manifest." },
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
      // Backdate a few hours apart so they sort into a natural order.
      const createdAt = new Date(
        now - (DEMO_POSTS.length - i) * 3 * 3_600_000,
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
