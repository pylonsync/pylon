# __APP_NAME__

A multi-tenant SaaS starter on [Pylon](https://pylonsync.com) — email/password
accounts, organizations with members + roles, and tenant-scoped data, all
server-rendered from one binary on one port. No Next.js, no separate API server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. Sign up, create an organization, and you land in a
workspace with **tenant-scoped projects** and a **members** panel. Create a
second org and switch between them — each org's projects are private to it.
Edit any file under `app/` and save — the page reloads instantly.

## Layout

```
app.ts                       User + Org/OrgMember/OrgInvite + tenant-scoped Project
app/page.tsx                 "/" — server-rendered, auth-aware homepage
app/login,signup/            email/password (POST /api/auth/password/*)
app/dashboard/               "/dashboard" — authed; org switcher + projects + members
app/dashboard/dashboard-client.tsx   the workspace client island
```

## How multi-tenancy works

Organizations are a **framework primitive**. Declaring `Org` / `OrgMember` /
`OrgInvite` with the framework's field names lights up `/api/auth/orgs/*`
(create/list orgs, members, invites) and `/api/auth/select-org` (switch your
active tenant) — driven by `<OrganizationSwitcher>` from `@pylonsync/client`.

`select-org` checks your `OrgMember` row before committing, then sets the
session's `tenantId`. Your data lives in tenant-scoped entities:

```ts
allowRead:   "auth.tenantId == data.orgId"
allowInsert: "auth.tenantId == data.orgId"
```

So `db.useQuery("Project")` returns only your **active org's** projects, and a
client literally cannot read or write another tenant's rows — switch orgs and
the list changes. **RBAC** is built in too: the framework gates invites/member
management to org admins, so a `member` calling `createInvite` gets a 403.

## Grow it

- **Add tenant data:** new `entity()` with an `orgId` + the same two policy
  lines. That's a new tenant-scoped table.
- **Custom roles:** read `OrgMember.role` in a server function and gate writes
  with `ctx.elevate({ admin })` / `ctx.db.unsafe.*`.
- **SSO / SAML:** per-org SSO is built in at `/api/auth/orgs/:id/sso/*`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
