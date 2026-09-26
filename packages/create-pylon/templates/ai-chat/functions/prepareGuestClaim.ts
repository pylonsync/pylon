import { mutation } from "@pylonsync/functions";
import { hashClaimCode, newClaimCode, CLAIM_TTL_MS } from "../lib/claim";

// Called by a guest right before signing in. Returns a one-time code that
// `claimGuestHistory` accepts after sign-in to move this guest's
// conversations to the account. Only the SHA-256 hash of the code is stored,
// and the code expires after 15 minutes.
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    const guestId = ctx.auth.userId;
    if (!ctx.auth.isGuest || !guestId) return { code: null };

    const code = newClaimCode();
    // GuestClaim is denied to clients in every direction by policy.
    const stale = (await ctx.db.unsafe.query("GuestClaim", { guestId })) as { id: string }[];
    for (const row of stale) await ctx.db.unsafe.delete("GuestClaim", row.id);
    // Claims from guests who never signed in expire; clear a batch of them.
    const expired = (await ctx.db.unsafe.query("GuestClaim", {
      expiresAt: { $lt: new Date().toISOString() },
      $limit: 50,
    })) as { id: string }[];
    for (const row of expired) await ctx.db.unsafe.delete("GuestClaim", row.id);
    await ctx.db.unsafe.insert("GuestClaim", {
      guestId,
      codeHash: await hashClaimCode(code),
      expiresAt: new Date(Date.now() + CLAIM_TTL_MS).toISOString(),
    });
    return { code };
  },
});
