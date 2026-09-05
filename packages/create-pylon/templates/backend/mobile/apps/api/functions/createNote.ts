import { mutation, v } from "@pylonsync/functions";
import { hasEntitlement } from "@pylonsync/revenuecat";
import { FREE_NOTE_LIMIT, PRO_ENTITLEMENT } from "../lib/purchases";

/**
 * Create a note for the caller. The free tier is capped here, on the
 * server, so the paywall cannot be skipped by writing the row directly
 * (the Note policy denies client inserts).
 *
 * Returns `LIMIT_REACHED` when a free account is at the cap; the app opens
 * the paywall on that code.
 */
export default mutation({
  args: {
    title: v.string(),
    body: v.optional(v.string()),
  },
  async handler(ctx, args: { title: string; body?: string }) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const title = args.title.trim();
    if (!title) throw ctx.error("INVALID_ARGS", "title is required");

    const entitlements = await ctx.db.query("RcEntitlement", { userId });
    const pro = hasEntitlement(
      entitlements as Array<{ entitlement: string; status: string; expiresAt?: string | null }>,
      PRO_ENTITLEMENT,
    );
    if (!pro) {
      const mine = await ctx.db.query("Note", { ownerId: userId });
      if (mine.length >= FREE_NOTE_LIMIT) {
        throw ctx.error(
          "LIMIT_REACHED",
          `Free accounts can keep ${FREE_NOTE_LIMIT} notes. Upgrade for unlimited notes.`,
        );
      }
    }
    return ctx.db.insert("Note", {
      ownerId: userId,
      title,
      body: args.body?.trim() ?? "",
    });
  },
});
