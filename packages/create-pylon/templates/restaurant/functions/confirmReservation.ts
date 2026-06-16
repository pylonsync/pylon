import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// confirmReservation — owner-only. Marks a pending reservation confirmed; the
// ReservationSlot marker stays, so the table remains held.
export default mutation<{ reservationId: string }, { ok: boolean }>({
  auth: "user",
  args: { reservationId: v.id("Reservation") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage reservations.");
    }
    await ctx.db.unsafe.update("Reservation", args.reservationId, { status: "confirmed" });
    return { ok: true };
  },
});
