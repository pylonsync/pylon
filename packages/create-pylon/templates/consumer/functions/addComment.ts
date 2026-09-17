import { mutation, v } from "@pylonsync/functions";
import { COMMENT_MAX } from "../lib/social";

/** Comment on a post. The post must exist and the text must fit. */
export default mutation<{ postId: string; text: string }, { id: string }>({
  // Guest sessions count: a visitor can post without signing up.
  auth: "guest",
  args: { postId: v.string(), text: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Open the app to start a session first.");
    const text = args.text.trim();
    if (!text) throw ctx.error("INVALID_COMMENT", "Write something first.");
    if (text.length > COMMENT_MAX) throw ctx.error("INVALID_COMMENT", `Use at most ${COMMENT_MAX} characters.`);
    const post = await ctx.db.get("Post", args.postId);
    if (!post) throw ctx.error("NOT_FOUND", "That post was deleted.");
    // The Comment policy refuses client inserts; this function is the
    // checked path, so it writes past the policy on purpose.
    const id = await ctx.db.unsafe.insert("Comment", {
      postId: args.postId,
      authorId: userId,
      text,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
