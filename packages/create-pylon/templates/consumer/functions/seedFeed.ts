import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Load the demo people, posts, likes, comments, and follows once. The feed
 * calls this on mount; it returns at once when the demo data exists, and a
 * lock keeps two first visits from seeding twice.
 *
 * Public so an anonymous first visitor seeds it. `unsafe.insert` writes past
 * the policies, which only accept the caller's own id, so the posts read as
 * twelve different people. It only writes lib/seed.ts. Delete this function
 * and lib/seed.ts when you launch.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "public",
  args: {},
  async handler(ctx) {
    await ctx.db.advisoryLock("consumer_seed_feed");
    const existing = await ctx.db.unsafe.query("Profile", { userId: "demo_mara_bakes", $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    for (const profile of seed.profiles) await ctx.db.unsafe.insert("Profile", profile);

    const postIds = new Map<string, string>();
    for (const post of seed.posts) {
      postIds.set(post.key, await ctx.db.unsafe.insert("Post", post.row));
    }
    for (const like of seed.likes) {
      await ctx.db.unsafe.insert("Like", { ...like.row, postId: postIds.get(like.post) });
    }
    for (const comment of seed.comments) {
      await ctx.db.unsafe.insert("Comment", { ...comment.row, postId: postIds.get(comment.post) });
    }
    for (const follow of seed.follows) await ctx.db.unsafe.insert("Follow", follow);
    return { seeded: true };
  },
});
