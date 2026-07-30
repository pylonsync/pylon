# __APP_NAME__

A polished, two-sided resale marketplace built with
[Pylon](https://pylonsync.com), with server-rendered product discovery and
realtime offers.

Anyone can list an item; anyone else can make an offer or buy it now; sellers
watch offers arrive and accept or decline them. Every write reaches each open
tab.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. The grid seeds itself on first load. Open a
second tab, post something from `/sell` in one, and watch it hit the "just
listed" ticker in the other with no refresh. Sign in (the `/sell` form prefills
a demo account) to list, save finds, make offers, and watch your dashboard
update live.

## How it works

- **Server-rendered for SEO + LCP.** The browse grid (`/`) and every listing
  page (`/listing/:slug`) render on the server with real rows (`serverData` +
  React 19 `use()`). View source and the products are *in the HTML*.
- **Realtime where it matters.** The "just listed" ticker, the live offers on a
  listing, and your `/me` dashboard all ride the sync engine: one `db.useQuery` per
  view, no polling. The public surface connects with an anonymous **guest
  session** (read-only); writing (list/offer/buy) requires a real sign-in.
- **Unspoofable ownership.** `sellerId`/`buyerId` use `field.owner()`, so the
  framework stamps them from the session and rejects forged values. Listings
  and offers can be created with a plain optimistic `db.insert` and still can't
  be spoofed. The heavier logic (accept = mark sold + auto-decline the rest)
  runs in `functions/respondToOffer.ts`, where it enforces "only the seller".
- **Commerce-ready discovery.** Browse by category, search title, description,
  seller, or category, and sort by recency or price. The logic is pure and
  covered by tests in `tests/example.test.ts`.

## Privacy and policies

- `Listing` + `Offer` are **public-read** (buyers and sellers both see live
  state); writes are owner-scoped.
- `Watch` (your saved listings) is **private**. You can only read and write your
  own rows.
- `User` rows are readable only to signed-in users (for seller/buyer names);
  `passwordHash` is `serverOnly` and never serialized. Auth writes go through
  `/api/auth/password/*`, not the entity API.

## Listing photos

Seeded listings include purpose-matched
[Unsplash](https://unsplash.com/license) photography. New listings collect a
JPG, PNG, or WebP upload (up to 8 MB), with a direct-link fallback and preview
before publishing. Uploads use Pylon's direct-to-storage three-step flow
(`/api/files/init` → signed upload URL → `/api/files/confirm`) so large files
do not pass through the app server. `imageUrl` stays optional so older rows
fall back to deterministic category artwork. The generated hero asset is a
44 KB WebP under `public/images`.

## Payments and fulfillment

`buyNow` atomically records an accepted offer, marks the listing sold, and
declines competing bids. It does not transfer money. Add your payment provider
and shipping or pickup workflow before accepting real transactions.

## Rebrand it

The brand ("Reprise") lives in `app/layout.tsx`; the demo catalog +
seed account are in `functions/seedMarket.ts`. The design tokens are in
`ui/tokens.css` + `app/globals.css`.

## Layout

```
app.ts                         User + Listing + Offer + Watch + policies
app/page.tsx                   SSR discovery, search, sort, category facets
app/listing/[id]/page.tsx      SSR listing detail (+ generateMetadata)
app/sell/page.tsx              list an item (sign-in gated)
app/me/page.tsx                your listings, offers, watchlist (live)
functions/buyNow.ts, makeOffer.ts, respondToOffer.ts, seedMarket.ts
client/*                       the realtime islands (ticker, offers, sell form…)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
