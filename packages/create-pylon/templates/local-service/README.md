# __APP_NAME__

A booking site for salons, barbers, trainers, clinics, trades, and other
appointment businesses. [Pylon](https://pylonsync.com) serves the marketing
page, live slot availability, and private owner dashboard from one process.

The time picker shows current availability. A slot greys out on every open page
as soon as someone books it.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and pick a service and time. Book a slot in one tab
and watch it grey out in another without refreshing.

## How the realtime works

- `app/booking-widget.tsx` subscribes to the public, PII-free `BookedSlot`
  projection with `db.useQuery`, so taken times grey out live across every tab.
- `functions/createBooking.ts` is a public **mutation** that re-checks the slot
  is still free (under a per-day advisory lock) before writing — so even a
  dead-heat double-click can't double-book. It writes the `Booking` (with the
  customer's details) and the `BookedSlot` (just the time range).
- `functions/cancelBooking.ts` deletes the `BookedSlot`, which frees the time
  on every open picker instantly.

## Privacy

The `Booking` entity holds the customer's name, email, and phone (PII), so its
policy in `app.ts` **denies every client read and write**. The public page only
ever reads `BookedSlot` — a name/email-free `{ startsAt, endsAt }` projection.
The full bookings, with contact details, are returned only by
`bookingsForOwner`, gated to the owner server-side. A booking site must never
leak its customers' contact details, and this is how that's guaranteed.

## The owner dashboard

`/dashboard` shows upcoming bookings grouped by day, with confirm/cancel and the
customer's contact details — updating live as bookings land and cancel.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`. Only that account can see
bookings.

## Rebrand + reconfigure it

Brand, colors, services, weekly hours, slot length, lead time, reviews,
location, and FAQ content live in **`lib/site.config.ts`**. Editing or
generating that file updates both the site and booking engine. Services and
hours stay in config rather than a separate database.

## Layout

```
app.ts                       data model (Booking, BookedSlot, User) + policies
lib/site.config.ts           ALL copy + brand + services + hours (edit this)
lib/slots.ts                 pure slot math, shared by picker + server re-check
lib/booking.ts               shared booking-row types
lib/owner.ts                 owner-email gate (PYLON_OWNER_EMAIL)
functions/createBooking.ts   public mutation: re-check + book (race-safe)
functions/bookingsForOwner.ts  owner-only query: bookings + customer PII
functions/{confirm,cancel}Booking.ts  owner-only mutations
app/page.tsx                 the landing page (server-rendered)
app/booking-widget.tsx       client island: live slot picker + booking form
app/login/page.tsx           owner sign-in
app/dashboard/               owner dashboard (auth-gated, live)
app/globals.css              Tailwind entrypoint (compiled by Pylon)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
