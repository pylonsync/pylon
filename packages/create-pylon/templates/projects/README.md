# __APP_NAME__

Client project delivery for a small team — projects, a task board, and time
logged against the work. One [Pylon](https://pylonsync.com) app: SSR frontend,
API, auth, and realtime sync from one process on one port.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and create an account. The first sign-in seeds two
live projects — one comfortably inside its budget, one over it — so both states
are visible rather than theoretical. Delete `functions/seedWorkspace.ts` and
`lib/seed.ts` once you have real work.

## What's interesting here

**Time is a ledger, not a total.** A task\'s logged hours are the sum of its
`TimeEntry` rows. Two people logging against the same task at once is normal,
and a mutable total loses one of them. It also means every hour on an invoice
traces to who logged it and when — which is the question a client asks.

Entries are **insert-only at the policy level**:

```ts
allowUpdate: "false",
allowDelete: "false",
```

Editing an entry would quietly change what someone was billed. A correction is a
negative entry, which stays visible in the history.

**Progress and budget are measured separately.** Progress counts tasks done;
budget tracks time spent. A project can be 90% through its budget and 20% done,
and a status report that conflates the two lies.

**The board is live.** Drag a task and it moves on every teammate\'s board
through the same write → sync → re-render path your own move takes, so a standup
doesn\'t start with reconciling two views of the work. The new position is
computed server-side, so two simultaneous drags don\'t claim the same index.

**Time input is lenient on purpose.** "90", "1.5h", "1h30", "45m" all work.
Rejecting "1h30" because it isn\'t "90" is the kind of friction that stops time
being logged at all, which costs far more than a forgiving parser. Values over a
day are refused — a typo on a timesheet becomes a typo on an invoice.

## Layout

```
app.ts                    entities + policies (note the append-only ledger)
app/
  page.tsx                "/" — the project list
  projects-view.tsx       list container
  workspace.tsx           app shell — the ONLY place that touches `db`
  projects/[id]/          task board, budget, time logging
  clients/                client list + billable rollups
  login/                  email/password auth
components/               presentational — props in, callbacks out
  ui/                     shadcn primitives
lib/work.ts               statuses, time, budget, progress — pure
lib/format.ts             time + initials — pure
functions/                server functions
tests/                    pure logic + component tests
```

Minutes are integers. Hours as floats produce 7.999999 on a timesheet, and
nobody wants to explain that.

## Access

Every entity is readable and writable by any **signed-in** user, and by nobody
else. Time entries are additionally insert-only for everyone.

There is no client portal. Adding one means a separate, narrower policy scoped
to that client\'s own projects, not loosening these.

## Grow it

- **Add a column:** `TASK_STATUSES` in `lib/work.ts`. The board, the validator,
  and the skeleton all read from it.
- **Invoice from time:** the billable rollup is already there
  (`billableCents`) — pair it with the `invoices` template.
- **Per-person timesheets:** group `TimeEntry` by `userId`; the rows already
  carry it.

## Deploy

```bash
pylon lint --strict
pylon test
pylon deploy
```

Docs: https://docs.pylonsync.com
