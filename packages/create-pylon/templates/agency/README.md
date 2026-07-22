# __APP_NAME__

A studio or agency site built with [Pylon](https://pylonsync.com). It combines
a server-rendered marketing page, live availability, a private project inquiry
form, and an owner dashboard in one server.

The hero shows how many project slots are open this quarter. When the owner
books a client from the dashboard, the count drops for every open page.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. In a second tab, sign in as the owner
(see below), book a lead from the dashboard, and watch the "N slots open" pill on
the public page tick down — with no refresh.

## How the realtime works

- `Capacity` is a public-read, PII-free row holding the booking window + open
  slot count. `app/contact-form.tsx` reads it with `db.useQuery("Capacity")`, so
  the hero's "N project slots open" pill is live everywhere.
- `functions/submitInquiry.ts` is a public **mutation** — anyone can send a
  project lead. A lead does not consume a slot because it is not a booking.
- `functions/bookInquiry.ts` / `declineInquiry.ts` are owner-only mutations that
  mark a lead booked or declined and adjust `Capacity.openSlots` under an advisory
  lock), so the public counter moves live.
- `functions/seedCapacity.ts` creates the Capacity row from config on first
  visit (idempotent).

## Privacy

The `Inquiry` entity holds the prospect's name, email, company, and budget (PII),
so its policy in `app.ts` **denies every client read and write**. The public page
only reads `Capacity` (a label + a number). Inquiries come back only through
`inquiriesForOwner`, gated to the owner server-side — the contact details never
travel over entity sync.

## The owner dashboard

`/dashboard` is the pipeline: every lead (with its details), Book / Decline /
Release actions, and a live availability editor (set the booking window + open
slots). It updates live as leads land.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`.

## Placeholders to replace

The page renders **clearly marked** placeholders anywhere a real photo belongs —
swap each for an `<img>` when you have the asset:

- the hero photo ("A photo of your studio or work")
- each case study's project shot (the `#work` grid)
- each team member's headshot

The logo cloud uses sample names — replace with your real client logos.

## Rebrand it

Brand, colors, hero copy, services,
case studies, process, team, testimonials, and the contact form's project-type
and budget options live in **`lib/site.config.ts`**. Edit that file, or have
Mast generate it, to update the studio. A fresh database seeds capacity from
the same config.

## Layout

```
app.ts                        Inquiry (PII) + Capacity (public, live) + User
lib/site.config.ts            ALL copy + brand + work/team/services (edit this)
functions/seedCapacity.ts     idempotent capacity seed from config
functions/submitInquiry.ts    public mutation: write a lead (PII), no slot change
functions/inquiriesForOwner.ts  owner-only query: leads + PII
functions/{book,decline}Inquiry.ts, setCapacity.ts  owner-only mutations
app/page.tsx                  the studio site (server-rendered)
app/contact-form.tsx          client island: live slots pill + inquiry form
app/dashboard/                owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
