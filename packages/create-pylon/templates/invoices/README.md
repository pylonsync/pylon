# __APP_NAME__

Invoicing for a small business — clients, line items, and payments. One
[Pylon](https://pylonsync.com) app: SSR frontend, API, auth, and realtime sync
from one process on one port.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and create an account. The first sign-in seeds a
realistic book — including one invoice already overdue and one part-paid — so
those states are visible rather than theoretical. Delete
`functions/seedWorkspace.ts` and `lib/seed.ts` once you're billing real clients.

## What's interesting here

**Money is integer cents, everywhere.** Floating-point dollars is how an invoice
ends up a penny off after tax, and a penny on a bill is a support ticket.
Quantities are thousandths for the same reason. Conversion happens only at the
edges — `parseAmount` on input, `money` on output.

**Totals and ageing are derived, never stored.** An invoice becomes overdue by
the clock passing, so nothing has to run at midnight to flip a column, and the
figure on screen always reconciles with the lines you can see. Tax is applied to
the rounded subtotal, which is what a customer can reproduce from the printed
invoice.

**The balance is computed server-side.** `recordPayment` recomputes from the
line items and existing payments before accepting anything — trusting a
client-sent balance would let a stale tab overpay, or mark an invoice settled
that isn't. Overpayment is rejected rather than silently producing a credit.

**It's live.** Recording a payment flips the balance and status on every open
tab, so two people chasing the same overdue invoice stop the moment one of them
marks it paid.

**Numbers come from the series, not a counter.** `nextNumber` derives the next
one from the existing rows: a counter in a synced replica is a race, and gaps in
an invoice series get asked about by accountants. `number` is unique in the
schema, so a genuine collision fails loudly.

## Layout

```
app.ts                    entities + policies
app/
  page.tsx                "/" — the invoice list
  invoices-view.tsx       list container
  workspace.tsx           app shell — the ONLY place that touches `db`
  invoices/[id]/          invoice detail, line items, payments
  clients/                client list
  login/                  email/password auth
components/               presentational — props in, callbacks out
  ui/                     shadcn primitives
lib/billing.ts            money, totals, ageing, numbering — pure
lib/format.ts             time + initials — pure
functions/                server functions
tests/                    pure logic + component tests
```

The data boundary lives in exactly one module (`app/workspace.tsx`). Everything
under `components/` takes data as props and reports changes through callbacks,
which is why the component tests render them with fixtures and mock nothing.

## A deliberate limitation

**A sent invoice can't have its lines edited.** It's a document someone is
paying against, and silently mutating it desyncs your copy from the one in their
inbox. Correct a sent invoice by voiding it and issuing another — the way paper
accounting has always worked, and the way an auditor expects.

## Access

Every entity is readable and writable by any **signed-in** user, and by nobody
else. There is **no client portal**, and no policy allows an anonymous read.
Adding one means a separate, narrower policy scoped to that client's own
invoices, not loosening these.

## Grow it

- **Change the tax model:** `totals` in `lib/billing.ts`. Per-line tax is a
  small change there and nowhere else.
- **Add a status:** `STATUSES` in the same file. Note that `overdue` is
  deliberately derived and not settable.
- **Email an invoice:** an action that renders the lines and sends through
  `ctx.email` — the totals helper already gives you every figure.

## Deploy

```bash
pylon lint --strict
pylon test
pylon deploy
```

Docs: https://docs.pylonsync.com
