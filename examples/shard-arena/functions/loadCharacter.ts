import { query, v } from "@pylonsync/functions";

/**
 * The character of user `userId`, for a zone the user joins: its id, x, the
 * number of its next grant, and its items. Zones call it (`Shard::calls`)
 * with the server's rights; a player cannot.
 */
export default query({
  args: { userId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.isAdmin) throw ctx.error("FORBIDDEN", "zones only");
    const userId = args.userId as string;
    const character = await ctx.db.lookup("Character", "userId", userId);
    if (!character) throw ctx.error("NOT_FOUND", `no character for ${userId}`);
    const items = await ctx.db.query("Item", { characterId: character.id });
    return {
      id: character.id,
      x: character.x,
      nextGrant: character.nextGrant,
      items: items.map((i) => ({ id: i.id, name: i.name })),
    };
  },
});
