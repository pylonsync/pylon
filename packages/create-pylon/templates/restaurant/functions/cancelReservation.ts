import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// cancelReservation — owner-only. Marks the reservation cancelled AND deletes
// its ReservationSlot marker, which FREES a table at that seating: the deletion
// syncs to every open picker, so "tables left" ticks back up live.
export default mutation<{ reservationId: string }, { ok: boolean }>({
  auth: "user",
  args: { reservationId: v.id("Reservation") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage reservations.");
    }
    await ctx.db.unsafe.update("Reservation", args.reservationId, { status: "cancelled" });

    const markers = (await ctx.db.unsafe.list("ReservationSlot")) as unknown as {
      id: string;
      reservationId: string;
    }[];
    const marker = markers.find((m) => m.reservationId === args.reservationId);
    if (marker) await ctx.db.unsafe.delete("ReservationSlot", marker.id);

    return { ok: true };
  },
});
