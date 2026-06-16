# __APP_NAME__

A small DTC store built with [Pylon](https://pylonsync.com) — a server-rendered
storefront with a **cart, real Stripe checkout, and live inventory**, plus a
private owner dashboard, all from one binary on one port. No Next.js, no
separate API server.

The realtime point: each product shows its stock, and the moment someone buys
the last unit it flips to "Sold out" for everyone with the page open.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Then **open a second tab**, add the last of an item
to your cart and check out in one, and watch it flip to "Sold out" in the
other — with no refresh.

> Checkout works with **zero config**: with no Stripe keys set, it holds the
> stock and records a "reserved" order for you to follow up on. Add
> `STRIPE_SECRET_KEY` (see [Checkout](#checkout)) to take real card payments.

## How the realtime works

- `Product` is a public-read entity holding live stock; `app/shop-client.tsx`
  reads it with `db.useQuery`, so every card's stock count is live.
- `functions/checkout.ts` is a public **action**. It calls
  `functions/reserveCart.ts` (an internal **mutation**) to re-check stock under a
  per-product advisory lock and HOLD it before the order is recorded — so two
  shoppers can't both buy the last unit. The stock change syncs to every open
  grid. With Stripe configured it then opens a hosted Checkout Session.
- `functions/cancelOrder.ts` returns units to stock (a sold-out item can come
  back live); `functions/restockProduct.ts` lets the owner add stock.
- `functions/seedProducts.ts` loads the catalog from config into the DB on first
  visit (idempotent).

## Privacy — read this

The `Order` entity holds the customer's name + email (PII), so its policy in
`app.ts` **denies every client read and write**. The public page only reads
`Product` (catalog + stock, no PII). Orders come back only through
`ordersForOwner`, gated to the owner server-side.

## The owner dashboard

`/dashboard` shows orders (with customer details), fulfill/cancel, and a live
stock table with one-tap restock — updating live as orders land.

Set `PYLON_OWNER_EMAIL` in `.env` (see `.env.example`) to the email you'll sign
in with, then create that account at `/login`.

## Checkout

Checkout is a single public `checkout` action that holds stock, then:

- **With `STRIPE_SECRET_KEY` set** → opens a hosted **Stripe Checkout** session
  (all cart lines priced inline from your catalog — no Stripe Products to set
  up) and redirects the shopper to it. The signed `stripeWebhook` action (at
  `/api/webhooks/stripeWebhook`) marks the order **paid** on success and
  **returns held stock** if a checkout is abandoned. The webhook signature is
  verified with `@pylonsync/stripe`'s constant-time verifier before any event is
  trusted.
- **Without Stripe keys** → the order is held as **reserved** for you to follow
  up on. The store still works end-to-end, so you can demo live inventory with
  zero setup.

See `.env.example` for the two env vars and the Stripe dashboard / `stripe
listen` setup.

## Rebrand it

Everything lives in **`lib/site.config.ts`** — brand, colors, the product list
(with starting stock), value props, reviews, policies. Edit that one file and
the whole store re-themes; the products re-seed on a fresh database.

## Layout

```
app.ts                     Product (public, live stock) + Order (PII) + User
lib/site.config.ts         ALL copy + brand + product catalog (edit this)
functions/seedProducts.ts  idempotent catalog seed from config
functions/checkout.ts      public action: hold stock + open Stripe Checkout
functions/reserveCart.ts   internal mutation: race-safe stock hold (per cart)
functions/stripeWebhook.ts public webhook: verify signature, settle the order
functions/{markGroupPaid,releaseGroup}.ts  internal: settle/restore on webhook
functions/ordersForOwner.ts  owner-only query: orders + customer PII
functions/{fulfill,cancel}Order.ts, restockProduct.ts  owner-only mutations
app/page.tsx               the storefront (server-rendered)
app/shop-client.tsx        client island: live product grid + cart + checkout
app/success/page.tsx       Stripe post-payment landing
app/dashboard/             owner dashboard (auth-gated, live)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
