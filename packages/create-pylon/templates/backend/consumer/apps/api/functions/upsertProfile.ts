import { mutation, v } from "@pylonsync/functions";
import { SEED_PEOPLE } from "../lib/seed";
import { BIO_MAX, HANDLE_PATTERN, findProfile, isImageUrl } from "../lib/social";

/**
 * Create or update the caller's Profile. Handles are lowercase and unique.
 * `avatarUrl` is optional; an empty string clears it.
 */
export default mutation({
	args: {
		handle: v.string(),
		displayName: v.string(),
		bio: v.string(),
		avatarUrl: v.optional(v.string()),
	},
	async handler(
		ctx,
		args: { handle: string; displayName: string; bio: string; avatarUrl?: string },
	) {
		const userId = ctx.auth.userId;
		const handle = args.handle.trim().toLowerCase();
		if (!HANDLE_PATTERN.test(handle)) {
			throw ctx.error(
				"INVALID_HANDLE",
				"Use 2 to 24 characters: lowercase letters, digits, periods, and underscores.",
			);
		}
		// The demo people in lib/seed.ts own these handles, even before the
		// seed runs. Delete this check with lib/seed.ts.
		if (SEED_PEOPLE.some((p) => p.handle === handle)) {
			throw ctx.error("HANDLE_TAKEN", `@${handle} is taken.`);
		}
		const displayName = args.displayName.trim();
		if (!displayName) throw ctx.error("EMPTY_NAME", "Enter your name.");
		if (displayName.length > 40) {
			throw ctx.error("NAME_TOO_LONG", "Use at most 40 characters for your name.");
		}
		const bio = args.bio.trim();
		if (bio.length > BIO_MAX) {
			throw ctx.error("BIO_TOO_LONG", `Use at most ${BIO_MAX} characters for your bio.`);
		}
		const avatarUrl = args.avatarUrl?.trim();
		if (avatarUrl && !isImageUrl(avatarUrl)) {
			throw ctx.error("INVALID_IMAGE", "Upload the photo again.");
		}

		// Two devices saving at once must not create two profiles.
		await ctx.db.advisoryLock(`profile:${userId}`);
		const existing = await findProfile(ctx, userId);
		const [taken] = await ctx.db.query("Profile", { handle, $limit: 1 });
		if (taken && taken.id !== existing?.id) {
			throw ctx.error("HANDLE_TAKEN", `@${handle} is taken.`);
		}

		// The Profile policy refuses client writes. This function is the
		// checked write path, so it writes past the policy.
		if (!existing) {
			const id = await ctx.db.unsafe.insert("Profile", {
				userId,
				handle,
				displayName,
				bio: bio || null,
				avatarUrl: avatarUrl || null,
				createdAt: new Date().toISOString(),
			});
			return await ctx.db.get("Profile", id);
		}
		await ctx.db.unsafe.update("Profile", existing.id, {
			handle,
			displayName,
			bio: bio || null,
			...(args.avatarUrl !== undefined ? { avatarUrl: avatarUrl || null } : {}),
		});
		return await ctx.db.get("Profile", existing.id);
	},
});
