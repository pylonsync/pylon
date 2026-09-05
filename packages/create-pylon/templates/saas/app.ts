import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";
// Per-workspace Stripe billing — see lib/billing.ts. `billing.manifest` brings
// the StripeSubscription entity + checkout/portal/cancel/restore/webhook actions
// + their read policy; the matching handlers live in functions/ (one wrapper per
// handler, re-exported from lib/billing.ts).
import { billing } from "./lib/billing";

// Accounts — email/password is built in (the entity named "User" is the
// account table; passwordHash is server-only). Each user can belong to many
// organizations.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    // The framework's /api/auth/password/register stamps a generated avatar
    // color here, so the User entity must declare it.
    avatarColor: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// ---------------------------------------------------------------------------
// Organizations — multi-tenancy is a framework primitive. Declaring these
// three entities with the names + fields below lights up the built-in
// `/api/auth/orgs/*` routes (create/list orgs, members, invites) and
// `/api/auth/select-org` (switch your active tenant). The framework writes
// only the fields it manages; add your own (logo, plan, billingEmail…) freely.
// The `@pylonsync/client` `<OrganizationSwitcher>` drives all of this for you.
// ---------------------------------------------------------------------------
const Org = entity(
  "Org",
  {
    name: field.string(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    // Stripe customer for this workspace's billing (referenceType: "org").
    // The billing plugin creates + stamps this on first checkout; server-only
    // so it never reaches the client.
    stripeCustomerId: field.string().serverOnly().optional(),
    // First-run state. `onboardedAt` is stamped when the /onboarding wizard
    // finishes (the dashboard sends a fresh workspace back there until then);
    // `setupDismissedAt` hides the Overview's getting-started checklist.
    onboardedAt: field.datetime().optional(),
    setupDismissedAt: field.datetime().optional(),
  },
  { indexes: [{ name: "by_created_by", fields: ["createdBy"], unique: false }] },
);

// User ↔ Org edge with a role. `select-org` checks this table before letting
// you switch tenants, so a client can't impersonate an org it doesn't belong
// to. role ∈ "owner" | "admin" | "member".
const OrgMember = entity(
  "OrgMember",
  {
    orgId: field.id("Org"),
    userId: field.id("User"),
    role: field.string(),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_user", fields: ["orgId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// Pending invite. The framework's /api/auth/orgs/:id/invites endpoints write
// these (tokenHash is server-only — the raw token only ever goes to the
// invitee). accepted* are filled in when the invite is redeemed.
const OrgInvite = entity(
  "OrgInvite",
  {
    orgId: field.id("Org"),
    email: field.string(),
    role: field.string(),
    invitedBy: field.id("User"),
    tokenHash: field.string().serverOnly(),
    tokenPrefix: field.string(),
    createdAt: field.datetime(),
    expiresAt: field.datetime(),
    acceptedAt: field.datetime().optional(),
    acceptedByUserId: field.id("User").optional(),
  },
  {
    indexes: [
      { name: "by_org", fields: ["orgId"], unique: false },
      { name: "by_email_org", fields: ["email", "orgId"], unique: false },
    ],
  },
);

// ---------------------------------------------------------------------------
// Your app's data — one tenant-scoped resource. `orgId` carries the tenant,
// and the policy scopes every read AND write to your ACTIVE org
// (`auth.tenantId`, set by select-org). Switch orgs in the UI and the project
// list changes — clients literally cannot read or write another tenant's rows.
// ---------------------------------------------------------------------------
const Project = entity(
  "Project",
  {
    orgId: field.id("Org"),
    name: field.string(),
    description: field.string().optional(),
    // "active" | "archived" — archived projects stay in the workspace but drop
    // out of the default view. Toggled from the Projects tab.
    status: field.string().default("active"),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_org", fields: ["orgId"], unique: false }] },
);

// User rows: read your own; the auth subsystem owns writes.
const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Org / OrgMember / OrgInvite are managed by the framework's /api/auth/orgs
// routes (which bypass these policies via the OrgStore). Clients reach them
// through the `@pylonsync/client` org helpers, not the entity API — so deny
// direct writes, and scope reads to your own membership / active org.
const orgPolicy = policy({
  name: "org_access",
  entity: "Org",
  allowRead: "auth.tenantId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});
const orgMemberPolicy = policy({
  name: "org_member_access",
  entity: "OrgMember",
  allowRead: "auth.userId == data.userId || auth.tenantId == data.orgId",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});
const orgInvitePolicy = policy({
  name: "org_invite_access",
  entity: "OrgInvite",
  allowRead: "auth.tenantId == data.orgId",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Projects are scoped to your ACTIVE tenant. `auth.tenantId == data.orgId`
// gates reads, edits, and deletes. Inserts go through the `createProject`
// function instead, which enforces the free plan's project cap on the server
// (lib/plans.ts) — a client cannot skip the paywall by writing the row itself.
const projectPolicy = policy({
  name: "project_tenant",
  entity: "Project",
  allowRead: "auth.tenantId == data.orgId",
  allowInsert: "false",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [User, Org, OrgMember, OrgInvite, Project, ...billing.manifest.entities],
  queries: fns.queries,
  // The billing actions (createCheckoutSession / createBillingPortalSession /
  // cancelSubscription / restoreSubscription / stripeWebhook) are re-exported
  // from functions/, so discovery already lists them; spreading
  // `billing.manifest.actions` as well would register each name twice and
  // the server refuses to boot on a duplicate. The plugin also declares
  // getSubscription/listSubscriptions queries, but the dashboard reads the
  // StripeSubscription entity directly (client-readable via the plugin's
  // policy), so those aren't wired.
  actions: fns.actions,
  policies: [
    userPolicy,
    orgPolicy,
    orgMemberPolicy,
    orgInvitePolicy,
    projectPolicy,
    ...billing.manifest.policies,
  ],
  // Email/password is on by default against the User entity. The org entities
  // above are named with the framework defaults (Org / OrgMember / OrgInvite),
  // so `/api/auth/orgs/*` + `/api/auth/select-org` work with no extra config.
  auth: auth(),
  // Self-hosted Inter (next/font parity): the build fetches the woff2, serves it
  // same-origin (no third-party request, no FOUT), preloads it, and synthesizes a
  // size-adjusted fallback face so there's no layout shift. globals.css reads it
  // via `var(--font-sans, …)`; layout.tsx carries no font <link>.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
