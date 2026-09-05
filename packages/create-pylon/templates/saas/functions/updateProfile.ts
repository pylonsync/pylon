import { mutation, v } from "@pylonsync/functions";

// The one User field a person may edit about themselves. Email changes
// need re-verification and passwords have their own route, so the User
// policy denies client writes and this function owns display-name edits.
export default mutation<{ displayName: string }, { displayName: string }>({
  args: { displayName: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "Sign in first.");
    const displayName = args.displayName.trim();
    if (displayName.length < 1 || displayName.length > 60) {
      throw ctx.error("INVALID_ARGS", "Name must be 1–60 characters.");
    }
    await ctx.db.unsafe.update("User", userId, { displayName });
    return { displayName };
  },
});
