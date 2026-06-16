import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// cancelBooking — owner-only. Marks the booking cancelled AND deletes its
// BookedSlot projection, which FREES the time: the deletion syncs to every open
// picker, so the slot becomes bookable again live.
export default mutation<{ bookingId: string }, { ok: boolean }>({
  auth: "user",
  args: { bookingId: v.id("Booking") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage bookings.");
    }
    await ctx.db.unsafe.update("Booking", args.bookingId, { status: "cancelled" });

    // Free the slot — find the projection row pointing at this booking.
    const slots = (await ctx.db.unsafe.list("BookedSlot")) as unknown as {
      id: string;
      bookingId: string;
    }[];
    const slot = slots.find((s) => s.bookingId === args.bookingId);
    if (slot) await ctx.db.unsafe.delete("BookedSlot", slot.id);

    return { ok: true };
  },
});
