# __APP_NAME__

A full-stack, multi-tenant SaaS starter on [Pylon](https://pylonsync.com),
branded as a fictional product called **Acme**: a server-rendered marketing
landing page, email/password auth, organizations with members + roles, and
tenant-scoped projects — all from one binary on one port. No Next.js, no
separate API server, no realtime sidecar.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. You get the **Acme landing page**. Sign up, create
an organization, and you land in a **workspace** with tenant-scoped projects and
a members panel. Create a second org and switch between them — each org's data
is private to it. Edit any file under `app/` and save — the page reloads.

## Layout

```
app.ts                       User + Org/OrgMember/OrgInvite + tenant-scoped Project
app/page.tsx                 "/" — the server-rendered Acme landing page (auth-aware)
app/layout.tsx               marketing nav + footer (rebrand "Acme")
app/login,signup/            email/password (POST /api/auth/password/*)
app/dashboard/               "/dashboard" — authed; org switcher + projects + members
app/dashboard/dashboard-client.tsx   the workspace client island
app/globals.css              Tailwind v4 + shadcn tokens (compiled by Pylon)
components/ui/                shadcn primitives (Button, Card)
```

## How it works

**The landing page** (`app/page.tsx`) is server-rendered React — view source and
the copy + SEO `<head>` are in the HTML, so it's fully indexable. It reads the
session during the render, so the call-to-action is "Get started" for visitors
and "Open dashboard" once you're signed in — no flash, no client fetch.

**Auth** is built in: `/login` + `/signup` POST to `/api/auth/password/*`, the
server sets an HttpOnly session cookie, and `/dashboard` redirects anonymous
visitors with a real 3xx before any HTML (works with JS off).

**Multi-tenancy** is a framework primitive. Declaring `Org` / `OrgMember` /
`OrgInvite` lights up `/api/auth/orgs/*` + `/api/auth/select-org`, driven by
`<OrganizationSwitcher>` from `@pylonsync/client`. Your data lives in
tenant-scoped entities (`Project`), gated by policy:

```ts
allowRead:   "auth.tenantId == data.orgId"
allowInsert: "auth.tenantId == data.orgId"
```

So `db.useQuery("Project")` returns only your **active org's** projects — switch
orgs and the list changes, and a client literally cannot read or write another
tenant's rows. `db.useQuery` is live; `db.insert` is optimistic.

## Make it yours

- **Rebrand:** replace "Acme" in `app/page.tsx` + `app/layout.tsx`.
- **Add tenant data:** new `entity()` with an `orgId` + the same two policy
  lines — a new tenant-scoped table, typed client and REST/realtime API included.
- **Add a route:** drop `app/about/page.tsx` and visit `/about`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
