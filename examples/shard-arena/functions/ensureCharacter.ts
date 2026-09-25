import { mutation } from "@pylonsync/functions";

/** Create the caller's character if they have none. */
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const existing = await ctx.db.lookup("Character", "userId", userId);
    const id = existing
      ? (existing.id as string)
      : await ctx.db.insert("Character", { userId, x: 0, nextGrant: 0 });
    return { characterId: id };
  },
});
