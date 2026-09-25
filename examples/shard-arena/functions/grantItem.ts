import { mutation, v } from "@pylonsync/functions";

/**
 * Grant `item` to a character. Zones call it with a key
 * (`loot:<character>:<n>`) and the host runs it once per key: a zone that
 * crashed and sends the grant again gets this call's result, and no second
 * item. `Item.grantKey` is unique as well. Zones only.
 *
 * `delayMs` (at most 10 s) holds the transaction open before it writes, so
 * tools/smoke-shard-cluster.sh can kill the machine in the middle.
 */
export default mutation({
  args: {
    characterId: v.string(),
    item: v.string(),
    key: v.string(),
    delayMs: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.isAdmin) throw ctx.error("FORBIDDEN", "zones only");
    const id = args.characterId as string;
    const key = args.key as string;
    const character = await ctx.db.get("Character", id);
    if (!character) throw ctx.error("NOT_FOUND", `no character ${id}`);
    const delay = Math.min(Number(args.delayMs ?? 0), 10_000);
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
    const itemId = await ctx.db.insert("Item", {
      characterId: id,
      name: args.item as string,
      grantKey: key,
    });
    const n = Number(key.split(":").pop());
    await ctx.db.update("Character", id, {
      nextGrant: Math.max(Number(character.nextGrant), n + 1),
    });
    return { itemId };
  },
});
