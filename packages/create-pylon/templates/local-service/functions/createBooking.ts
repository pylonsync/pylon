import { mutation, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";
import { rangesOverlap } from "../lib/slots";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

// createBooking — the ONLY way a Booking + its BookedSlot projection are
// written. It's a `mutation` (transactional, has `ctx.db`): one atomic write of
// the PII Booking row plus the PII-free BookedSlot, and the BookedSlot insert
// fires a change event that greys the slot out on every open picker in
// realtime.
//
// `auth: "public"` — a visitor has no account. The server RE-CHECKS that the
// slot is still free before inserting (the live UI mostly prevents collisions,
// but two people can hit "book" within the same instant). A per-day advisory
// lock serializes the check-then-insert so the re-check can't race — including
// on Postgres, where mutations can otherwise interleave.
//
// PRIVACY: returns only `{ ok, reason? }` — never a booking row or anyone's
// contact details.
export default mutation<
  {
    serviceSlug: string;
    startsAt: string;
    customerName: string;
    customerEmail: string;
    customerPhone?: string;
  },
  { ok: boolean; reason?: "past" | "taken" | "invalid" }
>({
  auth: "public",
  args: {
    serviceSlug: v.string(),
    startsAt: v.string(),
    customerName: v.string(),
    customerEmail: v.string(),
    customerPhone: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const svc = siteConfig.services.items.find((s) => s.slug === args.serviceSlug);
    if (!svc) throw ctx.error("INVALID_ARGS", "Unknown service.");

    const name = args.customerName.trim();
    const email = args.customerEmail.trim().toLowerCase();
    const phone = args.customerPhone?.trim() || null;
    if (name.length < 1 || name.length > 120) {
      throw ctx.error("INVALID_ARGS", "Enter your name.");
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      throw ctx.error("INVALID_ARGS", "Enter a valid email address.");
    }

    const startMs = Date.parse(args.startsAt);
    if (Number.isNaN(startMs)) return { ok: false, reason: "invalid" };
    const endMs = startMs + svc.durationMin * 60_000;
    const startsAt = new Date(startMs).toISOString();
    const endsAt = new Date(endMs).toISOString();

    // Must be far enough in the future.
    const minStart = Date.now() + siteConfig.booking.leadTimeHours * 3_600_000;
    if (startMs < minStart) return { ok: false, reason: "past" };

    // Serialize the check-then-insert for this calendar day so two concurrent
    // bookings can't both pass the overlap check. Held until this tx commits.
    await ctx.db.advisoryLock(`booking_day:${startsAt.slice(0, 10)}`);

    // Re-check overlap against existing busy slots (cross-user read → unsafe).
    const busy = (await ctx.db.unsafe.list("BookedSlot")) as unknown as {
      startsAt: string;
      endsAt: string;
    }[];
    const conflict = busy.some((b) =>
      rangesOverlap(startMs, endMs, Date.parse(b.startsAt), Date.parse(b.endsAt)),
    );
    if (conflict) return { ok: false, reason: "taken" };

    // Booking denies all client access by policy; createBooking is the only
    // writer, so these go through the trusted `unsafe` surface.
    const bookingId = await ctx.db.unsafe.insert("Booking", {
      serviceSlug: svc.slug,
      startsAt,
      endsAt,
      customerName: name,
      customerEmail: email,
      customerPhone: phone,
      status: "pending",
      createdAt: new Date().toISOString(),
    });
    await ctx.db.unsafe.insert("BookedSlot", {
      serviceSlug: svc.slug,
      startsAt,
      endsAt,
      bookingId,
    });

    return { ok: true };
  },
});
