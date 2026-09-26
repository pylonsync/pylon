# __APP_NAME__

A full-stack, multi-tenant SaaS starter on [Pylon](https://pylonsync.com),
branded as the fictional product **Acme**: project and task tracking for small
teams. It includes a server-rendered marketing site, first-run onboarding with
sample projects, email/password and Google auth, organizations with members
and roles, tenant-scoped projects and task boards that update live, and
per-workspace Stripe billing. Pylon serves it from one process.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 to load the Acme landing page. Sign up, name a
workspace, and pick **Start with sample projects** to land on a dashboard
with two projects and 16 tasks. Open a project board in two tabs and change a
task's status in one: the other updates within a second. Create a second
workspace and switch between them; each one's data stays private. Editing a
file under `app/` reloads the page.

## Layout

```
app.ts                          User + Org/OrgMember/OrgInvite + Project + Task + Stripe billing manifest
app/(marketing)/page.tsx        "/": the landing page (hero, product sections, quote, pricing, FAQ)
app/(marketing)/layout.tsx      marketing nav + footer
app/(marketing)/{products,solutions,resources,company,compare}/[slug]/   data-driven pages
app/(auth)/                     /login, /signup, forgot/reset password
app/onboarding/                 first-run wizard: workspace → invite → first project or samples → plan
app/dashboard/                  overview, projects, projects/[id] (task board), members, billing, settings
app/icon.svg, public/favicon.ico   the app icon
app/{robots,sitemap,llms}.ts    /robots.txt, /sitemap.xml, /llms.txt
functions/                      project + task mutations, seedWorkspace, onboarding, profile, Stripe handlers
lib/site.config.ts              every piece of marketing copy, the products, and the colors
lib/plans.ts                    prices, the free project cap, the trial length
lib/tasks.ts                    task statuses, priorities, validation, ordering
lib/sample-workspace.ts         the sample projects and tasks seedWorkspace adds
public/screenshots/             dashboard captures the landing page shows
scripts/capture-screenshots.mjs regenerates public/screenshots from the running app
components/                     dashboard shell, task board pieces, marketing pieces, shadcn primitives
```

## How it works

**The landing page** (`app/(marketing)/page.tsx`) is server-rendered React — view source and
the copy + SEO `<head>` are in the HTML, so it's fully indexable. It reads the
session during the render, so the call-to-action is "Get started" for visitors
and "Open dashboard" once you're signed in — no flash, no client fetch.

**Auth** is built in: `/login` + `/signup` POST to `/api/auth/password/*`, the
server sets an HttpOnly session cookie, and `/dashboard` redirects anonymous
visitors with a real 3xx before any HTML (works with JS off).

**Multi-tenancy** is a framework primitive. Declaring `Org` / `OrgMember` /
`OrgInvite` lights up `/api/auth/orgs/*` + `/api/auth/select-org`, driven by
`<OrganizationSwitcher>` from `@pylonsync/client`. Your data lives in
tenant-scoped entities (`Project`, `Task`), gated by policy:

```ts
allowRead:   "auth.tenantId == data.orgId"
```

So `db.useQuery("Project")` returns only your **active org's** projects, and
switching orgs changes the list. `Task` uses the same read policy.

Writes to projects and tasks go through server functions (`createProject`,
`setProjectStatus`, `deleteProject`, `createTask`, `updateTask`,
`deleteTask`). Each one loads the row, takes the workspace from the row, and
calls `ctx.requireMember` for that workspace. The entity policies deny direct
client writes, and `orgId` is `.readonly()`, so a client cannot move a row into
another workspace or skip the free plan's project cap.

## Make it yours

- **Rebrand and edit the copy:** everything the marketing site says, including
  the products, the quote, the FAQ, and the colors, lives in
  `lib/site.config.ts`. Edit one entry and the landing page, the nav, the
  footer, and the `[slug]` pages all follow. The quote and the "vs" pages are
  placeholders; replace them or delete them.
- **Replace the sample data:** edit `lib/sample-workspace.ts`, or remove the
  "Start with sample projects" option in `app/onboarding/onboarding-client.tsx`.
- **Refresh the screenshots:** after you change the dashboard, run
  `pylon dev`, then `node scripts/capture-screenshots.mjs` (it needs
  Playwright: `npm i -D playwright && npx playwright install chromium`). Use a
  fresh development database so the captures show only the sample workspace.
- **Add tenant data:** a new `entity()` with an `orgId: field.id("Org").readonly()`
  and the same read policy gives you a tenant-scoped table with a typed client
  and a REST and realtime API. Route its writes through functions that call
  `ctx.requireMember`, as `functions/createTask.ts` does.
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
