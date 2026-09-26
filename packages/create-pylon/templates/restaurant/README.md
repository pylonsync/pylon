# __APP_NAME__

A restaurant site built with [Pylon](https://pylonsync.com). One server handles
the menu, landing page, live table availability, and private owner dashboard.

Each seating shows how many tables remain. When someone takes the last table,
that time changes to "Full" on every open page.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and scroll to **Reserve**. Book the last table for a
time in one tab and watch it change to "Full" in another without refreshing.

## How the realtime works

- `app/(marketing)/reservation-widget.tsx` subscribes to the public, PII-free
  `ReservationSlot` markers with `db.useQuery` and COUNTS them per seating, so
  "tables left" ticks down live across every tab.
- `functions/createReservation.ts` is a public **mutation** that re-checks the
  seating is still under capacity (under a per-seating advisory lock) before
  writing — so two parties can't both grab the last table. It writes the
  `Reservation` (guest details) and a `ReservationSlot` marker (just the time).
- `functions/cancelReservation.ts` deletes the marker, which frees a table on
  every open picker instantly.

## Privacy

The `Reservation` entity holds the guest's name, email, phone, and notes (PII),
so its policy in `app.ts` **denies every client read and write**. The public
page only reads `ReservationSlot` — a bare `{ startsAt }` marker. The full
reservations come back only through `reservationsForOwner`, gated to the owner
server-side. A restaurant site must never leak its guests' contact details.

## The owner dashboard

`/dashboard` shows upcoming reservations grouped by day — party size, contact
details, notes — with confirm/cancel, plus live covers + counts.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`.

## Demo data

In `pylon dev`, an empty reservation book gets fictional tables from the past
week and the next two weeks (`functions/seedDemo.ts`, `lib/demo.ts`). Each one
writes a `Reservation` and its `ReservationSlot` marker, like a real
reservation, so the public calendar shows fewer tables left and the dashboard
lists the same guests. Seeding runs only when `PYLON_DEMO_DATA` is on, or when
it is unset and the app runs under `pylon dev`, and only while the `Reservation` table
is empty. `pylon start`, Docker, and Pylon Cloud deploys do not seed, because only `pylon dev` sets `PYLON_DEV_WATCH_DIR`. Set
`PYLON_DEMO_DATA=0` to start empty in development. Delete both files once you
take real reservations.

## Map or hours card

Set `location.mapEmbedUrl` in `lib/site.config.ts` to a Google Maps embed URL to
show a map beside the address. Without one, the site shows the week's service
hours (from `reservations.hours`, the same hours the calendar uses) and a
directions link. The dashboard reminds you while the URL is empty.

## Rebrand + reconfigure it

Brand, colors, the menu, seating hours, tables per seating, lead time, reviews,
location, and FAQ content live in **`lib/site.config.ts`**. Editing or
generating that file updates both the site and reservation engine.

The tab icon is `app/icon.svg` (with `public/favicon.ico` for browsers that
request `/favicon.ico`). Both carry this brand's letter and colors; replace
them when you rename the brand.

## Layout

```
app.ts                          Reservation + ReservationSlot + User + policies
lib/site.config.ts              ALL copy + brand + menu + seating config
lib/slots.ts                    pure seating math, shared by picker + server
lib/hours.ts                    weekly hours rows + directions link
lib/demo.ts                     demo reservations + the dev-only gate
functions/seedDemo.ts           dev-only demo reservations (see Demo data)
lib/reservation.ts              shared reservation-row types
lib/owner.ts                    owner-email gate (PYLON_OWNER_EMAIL)
functions/createReservation.ts  public mutation: capacity re-check + reserve
functions/reservationsForOwner.ts  owner-only query: reservations + guest PII
functions/{confirm,cancel}Reservation.ts  owner-only mutations
app/(marketing)/page.tsx        landing (hero + menu + reviews + location)
app/(marketing)/reservation-widget.tsx  client island: live table picker + form
app/dashboard/                  owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
