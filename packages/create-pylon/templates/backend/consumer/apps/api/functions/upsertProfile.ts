import { mutation, v } from "@pylonsync/functions";

/**
 * Create-or-update the caller's Profile. Handle is unique (case-
 * insensitive); we lowercase + check before inserting. On update
 * we don't re-check the handle if it didn't change.
 */
export default mutation({
	args: {
		handle: v.string(),
		displayName: v.string(),
		bio: v.string(),
	},
	async handler(
		ctx,
		args: { handle: string; displayName: string; bio: string },
	) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const handle = args.handle.trim().toLowerCase();
		if (!/^[a-z0-9_]{2,20}$/.test(handle)) {
			throw ctx.error(
				"INVALID_HANDLE",
				"handle must be 2–20 chars: lowercase letters, digits, underscore",
			);
		}
		const displayName = args.displayName.trim();
		if (!displayName) {
			throw ctx.error("EMPTY_NAME", "displayName cannot be empty");
		}

		const existing = (await ctx.db.query("Profile", {
			userId: ctx.auth.userId,
		})) as any[];

		if (existing.length === 0) {
			// New profile — handle must be unique
			const dup = (await ctx.db.query("Profile", { handle })) as any[];
			if (dup.length > 0) {
				throw ctx.error("HANDLE_TAKEN", `@${handle} is taken`);
			}
			const id = await ctx.db.insert("Profile", {
				userId: ctx.auth.userId,
				handle,
				displayName,
				bio: args.bio.trim() || null,
				createdAt: new Date().toISOString(),
			});
			return await ctx.db.get("Profile", id);
		}

		const profile = existing[0];
		// Update — re-check handle uniqueness only if changing
		if (profile.handle !== handle) {
			const dup = ((await ctx.db.query("Profile", { handle })) as any[]).filter(
				(p) => p.id !== profile.id,
			);
			if (dup.length > 0) {
				throw ctx.error("HANDLE_TAKEN", `@${handle} is taken`);
			}
		}
		await ctx.db.update("Profile", profile.id, {
			handle,
			displayName,
			bio: args.bio.trim() || null,
		});
		return await ctx.db.get("Profile", profile.id);
	},
});
