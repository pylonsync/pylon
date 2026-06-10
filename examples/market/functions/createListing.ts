import { mutation, v } from "@pylonsync/functions";

const PALETTE = ["a","b","c","d","e","f","g","h","i","j","k","l","m","n"];

/**
 * List an item for sale. The caller (a guest session) becomes the seller.
 */
export default mutation({
  // Public demo: any guest can sell. Without this the function defaults to
  // auth: "user" and rejects guest sessions.
  auth: "guest",
  args: {
    title: v.string(),
    description: v.string(),
    price: v.number(),
    category: v.string(),
    condition: v.string(),
    sellerName: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const title = args.title.trim();
    if (!title) throw ctx.error("INVALID_ARGS", "title is required");

    const seed =
      PALETTE[Math.floor(Math.random() * PALETTE.length)] +
      Math.random().toString(36).slice(2, 6);

    const id = await ctx.db.insert("Listing", {
      sellerId: ctx.auth.userId,
      sellerName: args.sellerName || "anonymous",
      title,
      description: args.description.trim(),
      price: Math.max(0, Math.round(args.price * 100) / 100),
      category: args.category || "other",
      condition: args.condition || "good",
      status: "active",
      seed,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
