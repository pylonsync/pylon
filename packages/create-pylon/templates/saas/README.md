# __APP_NAME__

A full-stack, multi-tenant SaaS starter on [Pylon](https://pylonsync.com),
branded as the fictional product **Acme**. It includes a server-rendered
marketing site, first-run onboarding, email/password and Google auth,
organizations with members and roles, tenant-scoped projects, and per-workspace
Stripe billing. Pylon serves it from one process.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 to load the Acme landing page. Sign up and create an
organization to enter a workspace with tenant-scoped projects and a members
panel. Create a second organization and switch between them; each one's data
remains private. Editing a file under `app/` reloads the page.

## Layout

```
app.ts                       User + Org/OrgMember/OrgInvite + Project + Stripe billing manifest
app/page.tsx                 "/" — the server-rendered Acme landing page (auth-aware)
app/layout.tsx               marketing nav + footer (rebrand "Acme")
app/{products,solutions,resources,company,compare}/[slug]/   data-driven marketing pages
app/login,signup/            email/password, email code, Google; forgot/reset password
app/onboarding/              first-run wizard: workspace → invite → first project → plan (trial)
app/pricing/                 monthly/annual plan table from lib/plans.ts
app/dashboard/               "/dashboard" — authed; overview + getting-started checklist, projects, members, billing, settings
app/dashboard/dashboard-client.tsx   the workspace client island
app/{error,not-found}.tsx    hydrated error + 404 boundaries
app/{robots,sitemap}.ts      /robots.txt + /sitemap.xml (enumerates every public page)
functions/                   createProject (free-plan cap), onboarding + profile mutations, Stripe handlers
lib/                         plans.ts (prices, cap, trial), billing.ts (@pylonsync/stripe), site.config.ts (copy)
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
- **Edit the marketing copy:** the product / solution / compare pages read from
  `lib/products.ts` + `lib/site.ts` — edit one entry and the nav dropdown, the
  footer, and the `[slug]` page all follow (they can't drift).
- **Add tenant data:** new `entity()` with an `orgId` + the same two policy
  lines — a new tenant-scoped table, typed client and REST/realtime API included.
- **Add a route:** drop `app/about/page.tsx` and visit `/about`.
- **Change prices or the free cap:** edit `lib/plans.ts`. The pricing page, the
  Billing tab, and the server-side cap in `functions/createProject.ts` all read it.
- **Enable billing:** set `STRIPE_SECRET_KEY`, `STRIPE_PRICE_PRO`, and
  `STRIPE_PRICE_PRO_ANNUAL` (see `.env.example`); the Billing tab then runs real
  Stripe Checkout (with the trial) + Customer Portal, kept in sync by the
  `/api/fn/stripeWebhook` handler.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
