import { mutation, v } from "@pylonsync/functions";
import { CAPTION_MAX, isPostImageUrl } from "../lib/social";

/**
 * Publish a photo. The client uploads the file first (uploadFile with
 * visibility "public", so every visitor can load it) and sends its URL here.
 * The URL, caption length, and crop shape are checked on the server.
 */
export default mutation<
  { imageUrl: string; caption?: string; shape?: string },
  { id: string }
>({
  // Guest sessions count: a visitor can post without signing up.
  auth: "guest",
  args: {
    imageUrl: v.string(),
    caption: v.optional(v.string()),
    shape: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Open the app to start a session first.");
    const imageUrl = args.imageUrl.trim();
    if (!isPostImageUrl(imageUrl)) throw ctx.error("INVALID_IMAGE", "Upload the photo again.");
    const caption = (args.caption ?? "").trim();
    if (caption.length > CAPTION_MAX) throw ctx.error("INVALID_CAPTION", `Use at most ${CAPTION_MAX} characters.`);
    const shape = args.shape === "portrait" ? "portrait" : "square";
    // The Post policy refuses client inserts; this function is the checked
    // path, so it writes past the policy on purpose.
    const id = await ctx.db.unsafe.insert("Post", {
      authorId: userId,
      imageUrl,
      caption: caption || null,
      shape,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
