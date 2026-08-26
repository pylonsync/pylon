import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// restaurant — a food & hospitality site with LIVE reservation availability.
// Same realtime engine as `local-service`, but capacity-based: each seating
// time has N tables, so the picker shows "4 left" and a time greys out for
// EVERYONE the instant the last table goes — no refresh. The server re-checks
// the count at insert time (under a per-slot lock) so two parties can't claim
// the last table at once.
//
//   • Reservation     — the real booking, with the guest's name/email/phone +
//                       party size. Holds PII → denies ALL client reads/writes.
//   • ReservationSlot — a PII-free `{ startsAt }` marker, one per reservation,
//                       PUBLIC-READ so the picker can COUNT how many tables are
//                       taken per seating and show what's left, live.
//   • User            — the owner's account (email/password) for the dashboard.
//
// Menu + seating hours + tables-per-seating are CONFIG (lib/site.config.ts).
// ---------------------------------------------------------------------------

const Reservation = entity(
  "Reservation",
  {
    startsAt: field.datetime(), // the seating time
    partySize: field.int(),
    customerName: field.string(),
    customerEmail: field.string(),
    customerPhone: field.string().optional(),
    notes: field.string().optional(),
    status: field.string().default("pending"), // "pending" | "confirmed" | "cancelled"
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_start", fields: ["startsAt"], unique: false }] },
);

// PII-free occupancy marker. One row per active reservation; the picker counts
// rows per `startsAt` to know how many tables are taken. Cancelling a
// reservation deletes its marker, which frees a table live.
const ReservationSlot = entity(
  "ReservationSlot",
  {
    startsAt: field.datetime(),
    reservationId: field.id("Reservation"),
  },
  { indexes: [{ name: "by_start", fields: ["startsAt"], unique: false }] },
);

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

// PRIVACY — Reservation holds the guest's contact details + notes, so it denies
// EVERY client read and write. The public page only ever reads ReservationSlot
// (a bare time marker), and the full reservations come back only through the
// owner-gated `reservationsForOwner`.
const reservationPolicy = policy({
  name: "reservation_private",
  entity: "Reservation",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Occupancy markers are public to READ (just a timestamp — the point is the
// live "tables left" count). Clients can't WRITE them; only createReservation /
// cancelReservation maintain them server-side.
const reservationSlotPolicy = policy({
  name: "reservation_slot_public_read",
  entity: "ReservationSlot",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Reservation, ReservationSlot, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [reservationPolicy, reservationSlotPolicy, userPolicy],
  auth: auth(),
  // Self-hosted Inter (next/font parity): the build fetches the woff2, serves it
  // same-origin (no third-party request, no FOUT), preloads it, and synthesizes a
  // size-adjusted fallback face so there's no layout shift. globals.css reads it
  // via `var(--font-sans, …)`; layout.tsx carries no font <link>.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
