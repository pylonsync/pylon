# Market — a live local marketplace

Buy and sell locally in realtime. Anyone can list an item or make an offer,
while sellers see offers arrive and accept or decline them. Every write reaches
each open tab.

The example combines server rendering and realtime sync:

- **Server-rendered for SEO + LCP.** The browse grid (`/`) and every listing
  page (`/listing/:id`) are rendered on the server with real rows from the
  database, via `serverData` + React 19 `use()`. View source and the products
  are *in the HTML* — not fetched after paint. `generateMetadata` reads the
  listing to produce a data-driven `<title>`/`<meta>` per page.
- **Realtime where it matters.** The "just listed" ticker, the live offers on
  a listing, and your `/me` inbox all ride the sync engine: a single
  `db.useQuery` per view, no polling. List something in one tab and watch it
  appear in another; make an offer and watch the seller's inbox light up.

`pylon dev` serves SSR, REST, and WebSockets from one binary on one port.

## Run it

```bash
pylon dev
```

Open <http://localhost:4321>. Browsing is public; **buying, selling, or
making an offer needs an account** using email and password. The example skips
verification email. A seeded demo shopper is prefilled in the login form:

```
demo@pylon.market  /  pylondemo123
```

The first visit seeds a catalog owned by a separate "Pylon Bazaar" seller
(so the demo shopper has plenty of other people's listings to buy), plus a
couple owned by the demo itself. Try it:

1. Sign in (prefilled) and open any Bazaar listing → **Buy now** for an
   instant purchase, or **Make an offer** to negotiate. Either way the buyer
   sees the result optimistically — no refresh.
2. For the seller side: open a second tab (incognito) and **Sign up**, then
   make an offer on one of the demo's own listings.
3. Back in the first tab, the offer appears live under **My Market** →
   **Accept**; the listing flips to *Sold* and the losing offers
   auto-decline, in both tabs at once.

## How it's wired

| Path | Rendering | What it shows |
|------|-----------|---------------|
| `/` | SSR grid (`serverData.query`) + live ticker island | Server data + realtime side by side |
| `/listing/:id` | SSR detail + dynamic `generateMetadata` + offers island | Data-driven SEO + the realtime centerpiece |
| `/sell` | client island | An optimistic `db.insert("Listing", …)` + client nav |
| `/me` | client island (3 live queries) | Your listings + offers, all live |

- **Auth** is built-in email/password — no verification email. `app.ts`
  declares a `User` entity (the `by_email` unique index + `passwordHash`
  serverOnly field is all the convention needs); registering through
  `/api/auth/password/register` hashes the password and writes the row.
  Browsing is public (`allowRead: "true"` on `Listing`/`Offer`); writing needs
  a real session.
- **Schema + policies** live in `app.ts`: `Listing` and `Offer`, public-read,
  owner-scoped writes.
- **Optimism is the default.** Posting a listing is a plain
  `db.insert("Listing", …)` — no server function. The row paints into the
  local store before the network round-trip finishes, so it appears in the
  "just listed" ticker immediately. It stays
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
  multi-row write. They default to `auth: "user"` — only signed-in members
  can bid or respond.
