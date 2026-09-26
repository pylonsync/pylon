import { mutation } from "@pylonsync/functions";
import {
	SEED_COMMENTS,
	SEED_PEOPLE,
	SEED_POSTS,
	photoPath,
	seedFollows,
	seedLikers,
} from "../lib/seed";

/**
 * Write the demo people, posts, likes, comments, and follows once. The
 * clients call this on launch. It returns at once when the demo data
 * exists, and a lock keeps two first launches from seeding twice.
 *
 * Public so the first launch can seed before anyone signs in. It writes
 * only the fixed data in lib/seed.ts. When you launch, delete this function,
 * lib/seed.ts, public/images, the reserved-handle check in upsertProfile.ts,
 * and the client calls to `seedDemo`.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
	auth: "public",
	args: {},
	async handler(ctx) {
		// The first demo account's email marks a seeded database. Check before
		// the lock so launches after the first skip it, then again under it.
		const marker = { email: `${SEED_PEOPLE[0].handle}@demo.invalid`, $limit: 1 };
		if ((await ctx.db.unsafe.query("User", marker)).length > 0) return { seeded: false };
		await ctx.db.advisoryLock("consumer_seed_demo");
		if ((await ctx.db.unsafe.query("User", marker)).length > 0) return { seeded: false };

		const now = Date.now();
		const hoursAgo = (h: number) => new Date(now - h * 3_600_000).toISOString();

		const profileIds = new Map<string, string>();
		for (const person of SEED_PEOPLE) {
			// A demo account has no password, so nobody can sign in as it.
			const userId = await ctx.db.unsafe.insert("User", {
				email: `${person.handle}@demo.invalid`,
				displayName: person.displayName,
				createdAt: hoursAgo(24 * 60),
			});
			const profileId = await ctx.db.unsafe.insert("Profile", {
				userId,
				handle: person.handle,
				displayName: person.displayName,
				bio: person.bio,
				createdAt: hoursAgo(24 * 60),
			});
			profileIds.set(person.handle, profileId);
		}

		const postIds = new Map<string, string>();
		const postAges = new Map<string, number>();
		for (const post of SEED_POSTS) {
			const id = await ctx.db.unsafe.insert("Post", {
				authorId: profileIds.get(post.author),
				imageUrl: photoPath(post.photo),
				caption: post.caption,
				createdAt: hoursAgo(post.age),
			});
			postIds.set(post.photo, id);
			postAges.set(post.photo, post.age);
			for (const [i, liker] of seedLikers(post.photo).entries()) {
				if (liker === post.author) continue;
				await ctx.db.unsafe.insert("Like", {
					postId: id,
					profileId: profileIds.get(liker),
					createdAt: hoursAgo(Math.max(post.age - 0.2 * (i + 1), 0.05)),
				});
			}
		}

		for (const comment of SEED_COMMENTS) {
			const postId = postIds.get(comment.photo);
			const age = postAges.get(comment.photo);
			if (!postId || age === undefined) continue;
			await ctx.db.unsafe.insert("Comment", {
				postId,
				profileId: profileIds.get(comment.author),
				text: comment.text,
				createdAt: hoursAgo(Math.max(age - comment.after, 0.05)),
			});
		}

		for (const [follower, following] of seedFollows()) {
			await ctx.db.unsafe.insert("Follow", {
				followerId: profileIds.get(follower),
				followingId: profileIds.get(following),
				createdAt: hoursAgo(24 * 30),
			});
		}
		return { seeded: true };
	},
});
