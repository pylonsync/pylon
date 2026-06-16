# __APP_NAME__

A studio / agency site built with [Pylon](https://pylonsync.com) — a
server-rendered marketing page with **live availability**, a private project
inquiry form, and an owner dashboard, all from one binary on one port. No
Next.js, no separate API server.

The realtime point: boutique studios take on a few projects at a time. The hero
shows how many slots are open this quarter, and the moment the owner books a new
client from the dashboard, that number drops for everyone with the page open.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Then **open a second tab**, sign in as the owner
(see below), book a lead from the dashboard, and watch the "N slots open" pill on
the public page tick down — with no refresh.

## How the realtime works

- `Capacity` is a public-read, PII-free row holding the booking window + open
  slot count. `app/contact-form.tsx` reads it with `db.useQuery("Capacity")`, so
  the hero's "N project slots open" pill is live everywhere.
- `functions/submitInquiry.ts` is a public **mutation** — anyone can send a
  project lead. It does NOT consume a slot (a lead isn't a booking).
- `functions/bookInquiry.ts` / `declineInquiry.ts` are owner-only mutations that
  mark a lead booked/declined AND adjust `Capacity.openSlots` (under an advisory
  lock), so the public counter moves live.
- `functions/seedCapacity.ts` creates the Capacity row from config on first
  visit (idempotent).

## Privacy — read this

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

Everything lives in **`lib/site.config.ts`** — brand, colors, hero, services,
case studies, process, team, testimonials, and the contact form's project-type
and budget options. Edit that one file (or have Mast generate it) and the whole
studio re-themes; the capacity re-seeds on a fresh database.

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
