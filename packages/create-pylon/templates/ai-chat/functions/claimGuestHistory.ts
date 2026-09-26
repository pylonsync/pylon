import { mutation, v } from "@pylonsync/functions";
import { hashClaimCode } from "../lib/claim";
import { isGuestId } from "../lib/chat";

// Move a guest's conversations to the signed-in account. The code comes from
// `prepareGuestClaim`, which only the guest session could call, so holding it
// proves the caller was that guest. Each code works once.
export default mutation({
  auth: "user",
  args: { code: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!/^[0-9a-f]{64}$/.test(args.code)) return { moved: 0 };

    // GuestClaim is server-only; the row is looked up by the code's hash.
    const [claim] = (await ctx.db.unsafe.query("GuestClaim", {
      codeHash: await hashClaimCode(args.code),
      $limit: 1,
    })) as { id: string; guestId: string; expiresAt: string }[];
    if (!claim) return { moved: 0 };
    await ctx.db.unsafe.delete("GuestClaim", claim.id);
    if (Date.parse(claim.expiresAt) < Date.now() || !isGuestId(claim.guestId)) return { moved: 0 };

    // Reassigns rows owned by the guest id the claim proved. Owner fields are
    // readonly to clients; this server-side update is the one path that
    // changes them.
    const conversations = (await ctx.db.unsafe.query("Conversation", {
      userId: claim.guestId,
    })) as { id: string }[];
    for (const c of conversations) {
      await ctx.db.unsafe.update("Conversation", c.id, { userId });
    }
    const messages = (await ctx.db.unsafe.query("Message", { userId: claim.guestId })) as { id: string }[];
    for (const m of messages) {
      await ctx.db.unsafe.update("Message", m.id, { userId });
    }
    return { moved: conversations.length };
  },
});
