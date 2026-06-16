# __APP_NAME__

A restaurant site built with [Pylon](https://pylonsync.com) — a server-rendered
menu + landing page with **live table availability** and a private owner
dashboard, all from one binary on one port. No Next.js, no separate API server.

The realtime point: every seating shows how many tables are left, and a time
greys out to "Full" for everyone the instant the last table goes.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321, scroll to **Reserve**. Then **open a second tab**,
book the last table at a time in one, and watch that time flip to "Full" in the
other — with no refresh.

## How the realtime works

- `app/reservation-widget.tsx` subscribes to the public, PII-free
  `ReservationSlot` markers with `db.useQuery` and COUNTS them per seating, so
  "tables left" ticks down live across every tab.
- `functions/createReservation.ts` is a public **mutation** that re-checks the
  seating is still under capacity (under a per-seating advisory lock) before
  writing — so two parties can't both grab the last table. It writes the
  `Reservation` (guest details) and a `ReservationSlot` marker (just the time).
- `functions/cancelReservation.ts` deletes the marker, which frees a table on
  every open picker instantly.

## Privacy — read this

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

## Rebrand + reconfigure it

Everything lives in **`lib/site.config.ts`** — brand, colors, the full menu,
seating hours, tables-per-seating, lead time, reviews, location, FAQ. Edit that
one file (or have a generator produce it) and the whole site — and the
reservation engine — reconfigures.

## Layout

```
app.ts                          Reservation + ReservationSlot + User + policies
lib/site.config.ts              ALL copy + brand + menu + seating config
lib/slots.ts                    pure seating math, shared by picker + server
lib/reservation.ts              shared reservation-row types
lib/owner.ts                    owner-email gate (PYLON_OWNER_EMAIL)
functions/createReservation.ts  public mutation: capacity re-check + reserve
functions/reservationsForOwner.ts  owner-only query: reservations + guest PII
functions/{confirm,cancel}Reservation.ts  owner-only mutations
app/page.tsx                    landing (hero + menu + reviews + location)
app/reservation-widget.tsx      client island: live table picker + form
app/dashboard/                  owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
