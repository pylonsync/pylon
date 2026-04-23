import { mutation, v } from "@pylonsync/functions";

/**
 * Update your own profile. Scoped to ctx.auth.userId — you can't edit anyone
 * else, which keeps the policy simple and the attack surface minimal.
 *
 * Email changes in a real app would trigger a re-verification flow; here
 * they're accepted immediately because the demo has no email gate.
 */
export default mutation({
  args: {
    displayName: v.optional(v.string()),
    email: v.optional(v.string()),
    avatarColor: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!me) throw ctx.error("USER_NOT_FOUND", "user row missing");

    const patch: Record<string, unknown> = {};

    if (args.displayName !== undefined) {
      const name = args.displayName.trim();
      if (name.length === 0 || name.length > 80) {
        throw ctx.error(
          "INVALID_NAME",
          "display name must be between 1 and 80 characters",
        );
      }
      if (name !== me.displayName) patch.displayName = name;
    }

    if (args.email !== undefined) {
      const email = args.email.trim().toLowerCase();
      if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
        throw ctx.error("INVALID_EMAIL", "invalid email address");
      }
      if (email !== me.email) patch.email = email;
    }

    if (args.avatarColor !== undefined) {
      // Light validation: must look like a hex color. Lets users pick
      // anything from a palette without locking them to a fixed list.
      const color = args.avatarColor.trim();
      if (!/^#?[0-9a-fA-F]{6}$/.test(color)) {
        throw ctx.error("INVALID_COLOR", "avatar color must be a 6-digit hex");
      }
      const normalized = color.startsWith("#") ? color : `#${color}`;
      if (normalized !== me.avatarColor) patch.avatarColor = normalized;
    }

    if (Object.keys(patch).length === 0) {
      return { userId: ctx.auth.userId, changed: false };
    }

    await ctx.db.update("User", ctx.auth.userId, patch);
    return { userId: ctx.auth.userId, changed: true };
  },
});
