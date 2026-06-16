import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { BookingRow, OwnerBookingsResult } from "../lib/booking";

// bookingsForOwner — the owner's view of every appointment, INCLUDING the
// customer name/email/phone. This is the one function allowed to return that
// PII, so it's gated to the configured owner (PYLON_OWNER_EMAIL via ctx.env).
//
// The dashboard calls it with `callFn` and re-fetches whenever the live, public
// BookedSlot set changes — so new bookings + cancellations show up without a
// refresh, while the contact details themselves never travel over entity sync.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerBookingsResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    // Booking denies all client reads; the owner-only full list goes through
    // the intentional cross-user read surface. Chronological by start.
    const rows = (await ctx.db.unsafe.list("Booking")) as unknown as BookingRow[];
    const bookings = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.startsAt < b.startsAt ? -1 : a.startsAt > b.startsAt ? 1 : 0));

    return { authorized: true, bookings };
  },
});
