# __APP_NAME__

A live, two-sided marketplace built with [Pylon](https://pylonsync.com), with
server-rendered listings and realtime offers.

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
a demo account) to list, make offers, and watch your `/me` inbox light up live.

## How it works

- **Server-rendered for SEO + LCP.** The browse grid (`/`) and every listing
  page (`/listing/:slug`) render on the server with real rows (`serverData` +
  React 19 `use()`). View source and the products are *in the HTML*.
- **Realtime where it matters.** The "just listed" ticker, the live offers on a
  listing, and your `/me` inbox all ride the sync engine — one `db.useQuery` per
  view, no polling. The public surface connects with an anonymous **guest
  session** (read-only); writing (list/offer/buy) requires a real sign-in.
- **Unspoofable ownership.** `sellerId`/`buyerId` use `field.owner()`, so the
  framework stamps them from the session and rejects forged values — listings
  and offers can be created with a plain optimistic `db.insert` and still can't
  be spoofed. The heavier logic (accept = mark sold + auto-decline the rest)
  runs in `functions/respondToOffer.ts` where it enforces "only the seller".

## Privacy and policies

- `Listing` + `Offer` are **public-read** (buyers and sellers both see live
  state); writes are owner-scoped.
- `Watch` (your saved listings) is **private** — read/write only your own rows.
- `User` rows are readable only to signed-in users (for seller/buyer names);
  `passwordHash` is `serverOnly` and never serialized. Auth writes go through
  `/api/auth/password/*`, not the entity API.

## Listing photos

Listings render a **deterministic gradient + category icon** from a `seed` —
so the demo needs no image hosting. To use real photos, add an `imageUrl` field
to `Listing` in `app.ts`, collect it in `client/SellForm.tsx` (upload via
`/api/files`), and render an `<img>` in the grid + detail page instead of the
gradient.

## Rebrand it

The brand ("Pylon Market") lives in `app/layout.tsx`; the demo catalog +
seed account are in `functions/seedMarket.ts`. The design tokens are in
`ui/tokens.css` + `app/globals.css`.

## Layout

```
app.ts                         User + Listing + Offer + Watch + policies
app/page.tsx                   SSR browse grid + category facets
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
