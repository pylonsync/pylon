import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// local-service — an appointment business (salon, barber, trainer, clinic,
// trades…) with LIVE slot availability. The realtime hook: the time picker
// subscribes to the day's booked slots, so the instant someone books, that
// slot greys out for everyone with the page open — no double-booking, no
// refresh. The server also re-checks at insert time to close the race.
//
// Two entities carry the booking; a third is the PII-free projection the public
// page is allowed to read:
//   • Booking    — the real appointment, with the customer's name/email/phone.
//                  Holds PII, so it denies ALL client reads/writes.
//   • BookedSlot — a name/email-free { startsAt, endsAt } projection of a busy
//                  slot, PUBLIC-READ so the picker can grey out taken times
//                  live. createBooking writes both; cancelling frees the slot.
//   • User       — the business owner's account (email/password), for the
//                  dashboard.
//
// Services and weekly hours are CONFIG (lib/site.config.ts) — the generator
// produces one typed file. Bookings reference a service by its config `slug`.
// ---------------------------------------------------------------------------

// The real appointment. `status` ∈ "pending" | "confirmed" | "cancelled".
// Everything except status is set once at booking time. Indexed by start so the
// owner dashboard + the server-side overlap re-check scan a day cheaply.
const Booking = entity(
  "Booking",
  {
    serviceSlug: field.string(),
    startsAt: field.datetime(),
    endsAt: field.datetime(),
    customerName: field.string(),
    customerEmail: field.string(),
    customerPhone: field.string().optional(),
    status: field.string().default("pending"),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_start", fields: ["startsAt"], unique: false }] },
);

// The PII-FREE projection of a busy slot. This is the ONLY booking data the
// public page reads — just the time range (and which service), never a name or
// email. The picker subscribes with `db.useQuery("BookedSlot")` so a newly
// taken slot greys out across every open tab through the replica. Cancelling a
// booking deletes its BookedSlot, which frees the time live.
const BookedSlot = entity(
  "BookedSlot",
  {
    serviceSlug: field.string(),
    startsAt: field.datetime(),
    endsAt: field.datetime(),
    bookingId: field.id("Booking"),
  },
  { indexes: [{ name: "by_start", fields: ["startsAt"], unique: false }] },
);

// The business owner's account (email/password is built in against "User").
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// PRIVACY — Booking holds the customer's name/email/phone, so it denies EVERY
// client read and write. No `db.useQuery("Booking")` can pull a row. Writes go
// only through the createBooking mutation; the owner reads them only through the
// owner-gated `bookingsForOwner` function. A booking site must never leak its
// customers' contact details.
const bookingPolicy = policy({
  name: "booking_private",
  entity: "Booking",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The busy-slot projection is public to READ (it's just a time range — the
// point is that the picker shows taken slots live to everyone). Clients can't
// WRITE it; only createBooking / cancelBooking maintain it server-side.
const bookedSlotPolicy = policy({
  name: "booked_slot_public_read",
  entity: "BookedSlot",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The owner reads their own User row (the owner gate resolves their email).
const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Booking, BookedSlot, User],
  // createBooking (public mutation), bookingsForOwner (owner query),
  // confirmBooking / cancelBooking (owner mutations) live in functions/ and are
  // discovered automatically.
  queries: [],
  actions: [],
  policies: [bookingPolicy, bookedSlotPolicy, userPolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
