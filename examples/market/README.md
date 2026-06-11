# Market — a live local marketplace

Buy and sell locally, in realtime. Anyone can list an item; anyone else can
make an offer; sellers watch offers arrive and accept or decline them — every
write fanned out to every open tab instantly.

It's the example that shows the two halves of Pylon working together:

- **Server-rendered for SEO + LCP.** The browse grid (`/`) and every listing
  page (`/listing/:id`) are rendered on the server with real rows from the
  database, via `serverData` + React 19 `use()`. View source and the products
  are *in the HTML* — not fetched after paint. `generateMetadata` reads the
  listing to produce a data-driven `<title>`/`<meta>` per page.
- **Realtime where it matters.** The "just listed" ticker, the live offers on
  a listing, and your `/me` inbox all ride the sync engine: a single
  `db.useQuery` per view, no polling. List something in one tab and watch it
  appear in another; make an offer and watch the seller's inbox light up.

One binary, one port. SSR + REST + WebSockets all from `pylon dev` — no
Next.js app, no separate realtime service.

## Run it

```bash
pylon dev
```

Open <http://localhost:4321>. The first visit seeds a dozen demo listings.
Open a second tab (or an incognito window for a second identity) and trade:

1. Tab A: open a listing, **Make an offer**.
2. Tab B (the seller): the offer appears live under **My Market** → **Accept**.
3. Both tabs update instantly — the listing flips to *Sold*, the losing
   offers are auto-declined.

## How it's wired

| Path | Rendering | What it shows |
|------|-----------|---------------|
| `/` | SSR grid (`serverData.query`) + live ticker island | Server data + realtime side by side |
| `/listing/:id` | SSR detail + dynamic `generateMetadata` + offers island | Data-driven SEO + the realtime centerpiece |
| `/sell` | client island | An optimistic `db.insert("Listing", …)` + client nav |
| `/me` | client island (3 live queries) | Your listings + offers, all live |

- **Schema + policies** live in `app.ts`: `Listing` and `Offer`, public-read,
  owner-scoped writes.
- **Optimism is the default.** Posting a listing is a plain
  `db.insert("Listing", …)` — no server function. The row paints into the
  local store instantly (it's in the "just listed" ticker before the network
  round-trip finishes) because that's how the sync engine works. It stays
  secure because `sellerId` is declared **`field.owner()`** in `app.ts`: the
  server stamps + verifies the owner from the session, so a forged seller id
  is rejected. Reach for `field.owner()` (instead of writing a function) any
  time the only server-authoritative part of a create is *who made it* —
  `authorId`, `buyerId`, `createdBy`.
- **Functions** (`functions/`) carry the logic that spans rows — e.g.
  `makeOffer` denormalizes the listing title + validates the listing is still
  active; `respondToOffer` marks the listing sold and declines the other
  offers in one mutation. Function calls (`db.useMutation`) take an
  `optimistic` builder when you want the same instant feedback for a
  multi-row write. They're declared `auth: "guest"` so the public demo works
  without a login (every visitor gets a guest session + a generated handle).
