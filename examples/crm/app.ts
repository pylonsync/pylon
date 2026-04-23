import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Attio-style CRM — companies, people, deals with polymorphic notes.
// Multi-tenant via Organization + OrgMember; everything else is org-scoped.
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  avatarColor: field.string(),
  createdAt: field.datetime(),
});

const Organization = entity("Organization", {
  name: field.string(),
  slug: field.string().unique(),
  createdBy: field.id("User"),
  createdAt: field.datetime(),
});

const OrgMember = entity(
  "OrgMember",
  {
    userId: field.id("User"),
    orgId: field.id("Organization"),
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

// ---- Core CRM objects -----------------------------------------------------

// Company represents an account. `customFieldsJson` keeps a JSON blob of
// arbitrary key/value pairs so users can add fields without a schema
// migration — small nod to Attio's runtime attributes without paying for
// the full runtime-schema machinery.
const Company = entity(
  "Company",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    domain: field.string().optional(),
    industry: field.string().optional(),
    sizeBucket: field.string().optional(), // 1-10 | 11-50 | 51-200 | 201-500 | 500+
    status: field.string(), // lead | active | churned
    description: field.string().optional(),
    ownerId: field.id("User").optional(),
    customFieldsJson: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_status", fields: ["orgId", "status"], unique: false },
      { name: "by_org_name", fields: ["orgId", "name"], unique: false },
    ],
  },
);

const Person = entity(
  "Person",
  {
    orgId: field.id("Organization"),
    firstName: field.string(),
    lastName: field.string().optional(),
    email: field.string().optional(),
    phone: field.string().optional(),
    title: field.string().optional(),
    companyId: field.id("Company").optional(),
    ownerId: field.id("User").optional(),
    customFieldsJson: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_company", fields: ["orgId", "companyId"], unique: false },
      { name: "by_org_email", fields: ["orgId", "email"], unique: false },
    ],
  },
);

// Deal is a sales opportunity. Stages are a small enum so the kanban
// view can group them. `amount` is the expected value in USD; probability
// is 0-100.
const Deal = entity(
  "Deal",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    companyId: field.id("Company").optional(),
    personId: field.id("Person").optional(),
    stage: field.string(), // lead | qualified | proposal | negotiation | won | lost
    amount: field.float(),
    probability: field.float(),
    closeDate: field.datetime().optional(),
    ownerId: field.id("User").optional(),
    description: field.string().optional(),
    customFieldsJson: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    updatedAt: field.datetime(),
    wonAt: field.datetime().optional(),
    lostAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_org_stage", fields: ["orgId", "stage"], unique: false },
      { name: "by_org_close", fields: ["orgId", "closeDate"], unique: false },
    ],
  },
);

// Polymorphic attachment. `targetType` picks the record kind (Company /
// Person / Deal); `targetId` points at the row. Notes are the primary way
// users add freeform context to a record.
const Note = entity(
  "Note",
  {
    orgId: field.id("Organization"),
    targetType: field.string(),
    targetId: field.string(),
    body: field.string(),
    authorId: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_target", fields: ["orgId", "targetType", "targetId"], unique: false },
    ],
  },
);

// Append-only activity log. UI renders it as a timeline; server writes
// one row every time a record is created, edited, or has a note.
const Activity = entity(
  "Activity",
  {
    orgId: field.id("Organization"),
    targetType: field.string(),
    targetId: field.string(),
    kind: field.string(), // created | updated | stage_changed | note_added
    metaJson: field.string().optional(),
    actorId: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_target", fields: ["orgId", "targetType", "targetId"], unique: false },
    ],
  },
);

// ---------------------------------------------------------------------------
// Policies — org-scoped row-level access via the new DSL.
// ---------------------------------------------------------------------------

const orgScoped = (name: string) =>
  policy({
    name: `${name}_org_scoped`,
    entity: name,
    allowRead: "auth.tenantId == data.orgId",
    allowInsert: "auth.tenantId == data.orgId",
    allowUpdate: "auth.tenantId == data.orgId",
    allowDelete: "auth.tenantId == data.orgId",
  });

const organizationPolicy = policy({
  name: "organization_access",
  entity: "Organization",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.createdBy",
  allowDelete: "auth.userId == data.createdBy",
});

const orgMemberPolicy = policy({
  name: "org_member_access",
  entity: "OrgMember",
  allowRead: "auth.userId == data.userId || auth.tenantId == data.orgId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const manifest = buildManifest({
  name: "crm",
  version: "0.1.0",
  entities: [User, Organization, OrgMember, Company, Person, Deal, Note, Activity],
  queries: [],
  actions: [],
  policies: [
    organizationPolicy,
    orgMemberPolicy,
    orgScoped("Company"),
    orgScoped("Person"),
    orgScoped("Deal"),
    orgScoped("Note"),
    orgScoped("Activity"),
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
