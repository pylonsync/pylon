# __APP_NAME__

A support inbox for a small team — tickets triaged by priority and SLA, with
shared threads and internal notes. One [Pylon](https://pylonsync.com) app: SSR
frontend, API, auth, and realtime sync from one process on one port.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and create an account. The first sign-in seeds a
realistic queue — including one urgent ticket already past its first-response
window, so the breach state is visible rather than theoretical. Delete
`functions/seedWorkspace.ts` and `lib/seed.ts` once real tickets arrive.

The tickets are spread across you and four demo teammates. Each is a User
row with a `@demo.invalid` address and no password, so outside dev mode nobody
can sign in as one (see `lib/demo-team.ts`). Delete those users along with the
seed.

## What's interesting here

**The queue answers "what next".** Rows are ordered by `queueOrder` in
`lib/tickets.ts` — breached first, then priority, then **oldest** first. Sorting
newest-first is how a ticket sits at the bottom of a busy queue for a week; this
ordering is pure and unit-tested.

**The SLA is computed, not stored.** Each priority carries a first-response
window; `slaState` compares it against `firstRespondedAt`. A ticket answered
late still counts as met — the question is "did we reply", and re-flagging
answered work only adds noise.

**It's live.** Assignment and replies reach every agent's screen through the
same write → sync → re-render path your own actions take, so two people don't
start the same reply.

**Internal notes are a first-class mode.** Sending one to the customer by
mistake is the expensive error here, so the composer changes colour and label,
the note stays marked in the thread, and — deliberately — an internal note does
**not** stop the SLA clock. A team that could clear its SLA by talking to itself
would have an SLA worth nothing.

## Brand and layout

The product ships as **Frontdesk**. The name lives in `lib/brand.ts`, and the
sidebar, sign-in screen, page titles, and phone drawer all read it. The mark is
`components/brand-mark.tsx`; `app/icon.svg`, `app/apple-icon.png`, and
`public/favicon.ico` draw the same shape for browsers and home screens. The
accent colour is `--brand` at the top of `app/globals.css`, and the typeface
(Public Sans) is set in `fonts` in `app.ts`.

The frame is `components/app-shell.tsx`. From 768px up the sidebar is a fixed
rail; below that it opens as a drawer from the menu button in each page header,
tables drop their lower-priority columns (`hideBelow` in
`components/data-table.tsx`), and boards scroll one column at a time. The nav
links are in `components/nav.tsx`. Every nav count comes from `navCounts` in
`app/workspace.tsx`, and page headers and tabs read the same numbers.

## Layout

```
app.ts                    entities + policies
app/
  page.tsx                "/" — the queue
  inbox-view.tsx          queue container (filters, new ticket)
  workspace.tsx           app shell — the ONLY place that touches `db`
  tickets/[id]/           ticket detail + thread
  customers/              customer list
  login/                  email/password auth
components/               presentational — props in, callbacks out
  ui/                     shadcn primitives
lib/tickets.ts            statuses, priority, SLA, queue order — pure
lib/format.ts             time + initials — pure
functions/                server functions
tests/                    pure logic + component tests
```

The data boundary lives in exactly one module (`app/workspace.tsx`). Everything
under `components/` takes data as props and reports changes through callbacks,
which is why the component tests render them with fixtures and mock nothing.

## Access

Every entity is readable and writable by any **signed-in** user, and by nobody
else — a shared queue is the point of a team helpdesk.

There is **no customer portal** here, and no policy allows an anonymous read.
Adding one means a separate, narrower policy scoped to the customer's own
tickets, not loosening these. Customers' email addresses live in `Customer`.

## Grow it

- **Change an SLA:** `PRIORITIES` in `lib/tickets.ts`. The queue order, the
  indicator, and the counts all read from it.
- **Add a status:** `STATUSES` in the same file; the filter and the detail-page
  picker follow automatically.
- **Take email in:** a `route.ts` webhook that creates a `Ticket` plus a
  `Message` with `fromCustomer: true` — see `functions/createTicket.ts` for the
  shape.

## Deploy

```bash
pylon lint --strict
pylon test
pylon deploy
```

Docs: https://docs.pylonsync.com
