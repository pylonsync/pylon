import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// confirmBooking — owner-only. Marks a pending booking confirmed. The BookedSlot
// already exists (createBooking wrote it), so the time stays held. Mutations DO
// have `ctx.error`, so the non-owner deny throws a typed error here.
export default mutation<{ bookingId: string }, { ok: boolean }>({
  auth: "user",
  args: { bookingId: v.id("Booking") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage bookings.");
    }
    await ctx.db.unsafe.update("Booking", args.bookingId, { status: "confirmed" });
    return { ok: true };
  },
});
