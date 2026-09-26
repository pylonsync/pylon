import { mutation, v } from "@pylonsync/functions";
import { CAPTION_MAX, isImageUrl, requireProfile } from "../lib/social";

/**
 * Publish a photo. The client uploads the file first (`/api/files/init`
 * with visibility "public", so every viewer can load it) and sends the
 * file URL here with the caption.
 */
export default mutation({
	args: { imageUrl: v.string(), caption: v.optional(v.string()) },
	async handler(ctx, args: { imageUrl: string; caption?: string }) {
		const profile = await requireProfile(ctx, ctx.auth.userId);
		const imageUrl = args.imageUrl.trim();
		if (!isImageUrl(imageUrl)) {
			throw ctx.error("INVALID_IMAGE", "Upload the photo again.");
		}
		const caption = (args.caption ?? "").trim();
		if (caption.length > CAPTION_MAX) {
			throw ctx.error("CAPTION_TOO_LONG", `Use at most ${CAPTION_MAX} characters.`);
		}
		// The Post policy refuses client writes. This function is the checked
		// write path, so it writes past the policy.
		const id = await ctx.db.unsafe.insert("Post", {
			authorId: profile.id,
			imageUrl,
			caption: caption || null,
			createdAt: new Date().toISOString(),
		});
		return { id };
	},
});
