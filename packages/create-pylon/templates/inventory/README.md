# __APP_NAME__

Stock control for a small business — products, an append-only movement ledger,
and the levels derived from it. One [Pylon](https://pylonsync.com) app: SSR
frontend, API, auth, and realtime sync from one process on one port.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and create an account. The first sign-in seeds a
realistic shelf — including one line out of stock and one below its reorder
point — so those states are visible rather than theoretical. Delete
`functions/seedWorkspace.ts` and `lib/seed.ts` once you stock real products.

## The one design decision that matters

**There is no `quantity` column.** On-hand is the SUM of a product\'s movements.

A mutable counter is the classic inventory bug: two people receive the same
delivery, both read 10, both write 15, and five units vanish with nothing to
audit. Appending `+5` twice gives 20 and shows exactly who did it.

It also makes every count explainable. "Why do we have 3?" is answerable from
the same rows that produced the number — the product page runs the balance
backwards so each movement shows what the shelf held right after it.

The ledger is **insert-only at the policy level**, not by convention:

```ts
allowUpdate: "false",
allowDelete: "false",
```

Editing a movement would silently rewrite a past valuation; deleting one would
make the current level unexplainable. A correction is a new movement in the
opposite direction — the way paper stock books have always worked.

## What else is interesting

**The reason has to agree with the direction.** A "Sold" that adds stock is a
mis-click, and `recordMovement` refuses it. The whole value of storing a reason
is being able to trust it later.

**You type positive numbers.** The sign comes from the reason. Asking someone to
type `-3` for a sale is how you get a `+3` sale and a shelf that disagrees with
the system. A stock count is the exception: you type what you counted, and the
delta is computed from the current level.

**Negative stock is refused.** It\'s always a miscount or a mistake, and
recording it turns one error into a valuation that quietly lies. Correct it with
a stock count instead.

**It\'s live.** A movement recorded at the back door updates the level on the
shop floor\'s screen immediately, so nobody sells what was just damaged.

## Layout

```
app.ts                    entities + policies (note the append-only ledger)
app/
  page.tsx                "/" — the stock list
  products-view.tsx       list container
  workspace.tsx           app shell — the ONLY place that touches `db`
  products/[id]/          product detail + running balance history
  movements/              the full ledger
  login/                  email/password auth
components/               presentational — props in, callbacks out
  ui/                     shadcn primitives
lib/stock.ts              levels, valuation, reorder state — pure
lib/format.ts             time + initials — pure
functions/                server functions
tests/                    pure logic + component tests
```

Money is integer cents; quantities are whole units. You cannot hold half a
physical thing, and allowing it hides unit-of-measure mistakes.

## Access

Every entity is readable by any **signed-in** user and by nobody else. Movements
are additionally insert-only for everyone, including you.

## Grow it

- **Add a reason:** `REASONS` in `lib/stock.ts`, with its allowed direction. The
  dialog, the validator, and the ledger view all read from it.
- **Multiple locations:** add a `locationId` to `Movement` and group the sum by
  it — the model already supports it without a schema migration on levels,
  because there are no levels to migrate.
- **Purchase orders:** a `PurchaseOrder` entity whose receipt writes movements.

## Deploy

```bash
pylon lint --strict
pylon test
pylon deploy
```

Docs: https://docs.pylonsync.com
