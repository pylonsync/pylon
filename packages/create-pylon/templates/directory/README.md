# __APP_NAME__

A curated directory built with [Pylon](https://pylonsync.com), with live
full-text search, facets, community upvotes, and moderated submissions.

The browse page runs `db.useSearch` against the listing table. Search results,
facet counts, and votes update across every open tab.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. The directory seeds itself on first load. Search,
filter by category, and upvote, then open a second tab to watch vote counts sync.

## How it works

- **Live faceted search.** `app/directory-browse.tsx` calls
  `db.useSearch("Listing", { query, filters, facets, sort })`, declared by the
  `search: { text, facets, sortable }` block on the `Listing` entity in `app.ts`
  (Pylon builds FTS5 + facet shadow tables). It re-runs on every keystroke AND
  whenever the table is written — so it doubles as the realtime layer.
- **Live upvotes.** The public `upvote` mutation bumps `Listing.votes` under a
  per-listing advisory lock; because `useSearch` is live, the new count appears
  in every open tab instantly.
- **Moderated submissions.** `submitListing` (public) writes a pending
  `Submission`; the curator approves it from `/dashboard`, which copies the
  public fields into a new `Listing` (`approveSubmission`).

## Privacy

The `Submission` entity holds the submitter's name + email (PII), so its policy
in `app.ts` **denies every client read and write**. The public directory only
reads `Listing` (no PII). Submissions come back only through the owner-gated
`submissionsForOwner`, and `approveSubmission` copies *only* the PII-free fields
into the public `Listing` — the submitter's contact details never become public.

## The curator dashboard

`/dashboard` is the moderation queue: pending submissions (with submitter
details), Approve / Reject, and a live count of published listings.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`.

## Demo data

The curator's first dashboard load in `pylon dev` fills an empty review queue
with fictional submissions (`functions/seedDemo.ts`, `lib/demo.ts`). Seeding
runs only for the owner, only when `PYLON_DEMO_DATA` is on or when it is unset and the app runs under `pylon dev`, and only while the `Submission` table is empty. `pylon start`, Docker, and Pylon Cloud deploys do not seed, because only `pylon dev` sets `PYLON_DEV_WATCH_DIR`. Set `PYLON_DEMO_DATA=0` to start
empty in development.

The starter listings seed in every environment (`functions/seedListings.ts`),
because they are your directory's own content from `lib/site.config.ts`. Their
names are made up; replace them with real entries. Each listing shows a
monogram mark (`monogram` in `lib/directory.ts`) until you add logos.

## Rebrand it

Brand, colors, hero copy, categories, starter listings, and submission copy
live in **`lib/site.config.ts`**. Editing that file updates the directory, and
a fresh database seeds from its starter listings.

The tab icon is `app/icon.svg` (with `public/favicon.ico` for browsers that
request `/favicon.ico`). Both carry this brand's letter and colors; replace
them when you rename the brand.

## Layout

```
app.ts                          Listing (public, FTS) + Submission (PII) + User
lib/site.config.ts              ALL copy + brand + categories + seed listings
functions/seedListings.ts       idempotent seed from config
functions/seedDemo.ts           dev-only demo review queue (see Demo data)
lib/directory.ts                shared types, tags, monogram marks
lib/demo.ts                     demo submissions + the dev-only gate
functions/submitListing.ts      public mutation: write a pending Submission (PII)
functions/upvote.ts             public mutation: bump Listing.votes (live)
functions/submissionsForOwner.ts  owner-only query: queue + submitter PII
functions/{approve,reject}Submission.ts  owner-only moderation
app/(marketing)/page.tsx        headline + browse island
app/(marketing)/directory-browse.tsx        client island: live db.useSearch + facets + votes
app/(marketing)/submit/page.tsx, submit-form.tsx  the submit flow
app/dashboard/                  curator moderation queue (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
