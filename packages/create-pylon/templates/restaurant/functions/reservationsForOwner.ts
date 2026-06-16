import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { ReservationRow, OwnerReservationsResult } from "../lib/reservation";

// reservationsForOwner — the owner's view of every reservation, INCLUDING the
// guest's name/email/phone + notes. The one function allowed to return that
// PII, gated to the configured owner (PYLON_OWNER_EMAIL via ctx.env).
//
// The dashboard calls it with `callFn` and re-fetches whenever the live, public
// ReservationSlot set changes — so new reservations + cancellations show up
// without a refresh, while contact details never travel over entity sync.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerReservationsResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Reservation")) as unknown as ReservationRow[];
    const reservations = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.startsAt < b.startsAt ? -1 : a.startsAt > b.startsAt ? 1 : 0));

    return { authorized: true, reservations };
  },
});
