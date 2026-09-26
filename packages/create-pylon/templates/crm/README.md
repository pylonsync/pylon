# __APP_NAME__

A sales CRM for a small team — companies, contacts, and deals on a live
pipeline board. One [Pylon](https://pylonsync.com) app: SSR frontend, API, auth,
and realtime sync from one process on one port.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and create an account. The first sign-in seeds a demo
pipeline so the board isn't empty; delete `functions/seedWorkspace.ts` and
`lib/seed.ts` once you have real customers.

The deals are spread across you and four demo teammates. Each is a User
row with a `@demo.invalid` address and no password, so outside dev mode nobody
can sign in as one (see `lib/demo-team.ts`). Delete those users along with the
seed.

## What's interesting here

**The board is live.** Drag a deal to another column and it moves on every
teammate's board immediately — no refresh, no polling. Your own move takes the
same path as theirs: write → sync → re-render, so there's one behaviour to
reason about instead of two. Open the app in two windows and try it.

**⌘K searches everything.** Deals, companies, and contacts, ranked by how well
they match (`lib/search.ts`). It runs against the synced replica in memory, so
results appear as you type with no request per keystroke. `/` opens it too, and
`c` creates a deal.

**The forecast is derived, never stored.** Open pipeline, weighted value, and
win rate are computed from the deals themselves in `lib/pipeline.ts` — pure
functions with no React and no `db`, so the arithmetic is unit-tested directly
(`tests/pipeline.test.ts`).

## Brand and layout

The product ships as **Dealbook**. The name lives in `lib/brand.ts`, and the
sidebar, sign-in screen, page titles, and phone drawer all read it. The mark is
`components/brand-mark.tsx`; `app/icon.svg`, `app/apple-icon.png`, and
`public/favicon.ico` draw the same shape for browsers and home screens. The
accent colour is `--brand` at the top of `app/globals.css`, and the typeface
(Instrument Sans) is set in `fonts` in `app.ts`.

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
  page.tsx                "/" — the pipeline board
  pipeline-view.tsx       board container
  workspace.tsx           app shell — the ONLY place that touches `db`
  companies/, contacts/   list views
  deals/[id]/             deal detail + activity timeline
  login/                  email/password auth
components/               presentational — props in, callbacks out
  ui/                     shadcn primitives
lib/pipeline.ts           stages, money, metrics — pure
lib/search.ts             ⌘K ranking — pure
functions/                server functions
tests/                    pure logic + component tests
```

The data boundary lives in exactly one module (`app/workspace.tsx`). Everything
under `components/` takes data as props and reports changes through callbacks,
which is why the component tests render them with fixtures and mock nothing.

## Access

Every entity is readable and writable by any **signed-in** user, and by nobody
else — a shared pipeline is the point of a team CRM. `ownerId` records who
created a row without partitioning access.

For per-rep isolation, tighten `allowRead` in `app.ts`:

```ts
allowRead: "auth.userId == data.ownerId"
```

Contacts hold customer email and phone. There is no public route in this app and
no policy allows an anonymous read — keep it that way if you add one.

## Theme

Light by default. The dark tokens in `app/globals.css` are complete, so adding
`className="dark"` to the `<html>` in `app/layout.tsx` flips the whole app —
wire it to a stored preference or `prefers-color-scheme` for a toggle. Both use
the standard shadcn variables, so `npx shadcn@latest add <component>` drops in
already themed.

## Grow it

- **Add a field:** edit an entity in `app.ts` — the typed client and
  REST/realtime API follow, no migration.
- **Add a stage:** `PIPELINE` in `lib/pipeline.ts`. The board, the forecast, and
  the stage picker all read from it.
- **Add a view:** drop `app/reports/page.tsx` and add it to `NAV` in
  `components/nav.tsx`.

## Deploy

```bash
pylon lint --strict
pylon test
pylon deploy
```

Docs: https://docs.pylonsync.com
