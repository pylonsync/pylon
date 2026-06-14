import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

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
// gates read AND write — and because orgId is client-supplied at insert time
// (not stamped later), checking it here means you can only create a project in
// the org you've selected. Switch orgs → a different project list.
const projectPolicy = policy({
  name: "project_tenant",
  entity: "Project",
  allowRead: "auth.tenantId == data.orgId",
  allowInsert: "auth.tenantId == data.orgId",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [User, Org, OrgMember, OrgInvite, Project],
  queries: [],
  actions: [],
  policies: [
    userPolicy,
    orgPolicy,
    orgMemberPolicy,
    orgInvitePolicy,
    projectPolicy,
  ],
  // Email/password is on by default against the User entity. The org entities
  // above are named with the framework defaults (Org / OrgMember / OrgInvite),
  // so `/api/auth/orgs/*` + `/api/auth/select-org` work with no extra config.
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
