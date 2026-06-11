import { mutation, v } from "@pylonsync/functions";

export default mutation({
  auth: "guest",
  args: {
    email: v.string(),
    message: v.optional(v.string()),
    source: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const email = args.email.trim().toLowerCase();
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      throw ctx.error("INVALID_ARGS", "Please enter a valid email address.");
    }
    const message = (args.message ?? "").trim();
    const source = (args.source ?? "website").trim();
    await ctx.db.insert("Lead", {
      email,
      source,
      message,
      createdAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
