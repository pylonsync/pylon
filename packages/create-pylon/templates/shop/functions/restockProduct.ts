import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// restockProduct — owner-only. Adds units to a product's stock. The bump syncs
// to every open grid, so the shelf updates live (a sold-out item reappears).
export default mutation<{ slug: string; add: number }, { ok: boolean; stock: number }>({
  auth: "user",
  args: { slug: v.string(), add: v.int() },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage stock.");
    }
    const add = Math.max(0, Math.trunc(args.add));
    const product = (await ctx.db.unsafe.lookup("Product", "slug", args.slug)) as
      | { id: string; stock: number }
      | null;
    if (!product) throw ctx.error("NOT_FOUND", "Product not found.");
    const stock = product.stock + add;
    await ctx.db.unsafe.update("Product", product.id, { stock });
    return { ok: true, stock };
  },
});
