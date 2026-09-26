import { query } from "@pylonsync/functions";
import { findProfile } from "../lib/social";

/**
 * The caller's Profile, or null when they have not made one yet. The
 * clients call this after sign-in to decide between the profile setup
 * screen and the feed.
 */
export default query({
	args: {},
	async handler(ctx) {
		return await findProfile(ctx, ctx.auth.userId);
	},
});
