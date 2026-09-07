import { mutation, v } from "@pylonsync/functions";
import { ENTITLEMENT_ENTITY } from "../lib/purchases";

/**
 * Deletes the caller's rows before Pylon deletes the account. Wired
 * through `auth({ onDeleteAccount: "deleteMyData" })` in app.ts; the
 * runtime calls it as the user with `{ userId }` and aborts the deletion
 * if it throws. `internal: true` keeps it off the HTTP surface.
 *
 * Add every entity that stores user data here. Pylon removes the User
 * row, sessions, API keys, linked accounts, and trusted devices; nothing
 * else.
 */
export default mutation({
  internal: true,
  auth: "guest",
  args: { userId: v.string() },
  async handler(ctx, args: { userId: string }) {
    if (ctx.auth.userId !== args.userId) {
      throw ctx.error("FORBIDDEN", "a user can only delete their own data");
    }

    const notes = await ctx.db.query("Note", { ownerId: args.userId });
    for (const note of notes) {
      await ctx.db.delete("Note", String(note.id));
    }

    // The entitlement policy denies every client delete. This is server
    // code running as part of account deletion, so the unsafe surface
    // (policy bypass) is the right tool.
    const entitlements = await ctx.db.query(ENTITLEMENT_ENTITY, { userId: args.userId });
    for (const row of entitlements) {
      await ctx.db.unsafe.delete(ENTITLEMENT_ENTITY, String(row.id));
    }

    return { notes: notes.length, entitlements: entitlements.length };
  },
});
