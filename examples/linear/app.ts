import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Linear-style issue tracker. Organization → Team(s) → Issue, with cycles,
// projects, labels, comments, and a per-team monotonic issue number.
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

// Teams are the unit issue numbering hangs off — "ENG-1", "DESIGN-42", etc.
const Team = entity(
  "Team",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    key: field.string(), // "ENG" — short prefix for issue numbers
    description: field.string().optional(),
    issueSequence: field.float(), // bumped on each issue create
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_org_key", fields: ["orgId", "key"], unique: true }],
  },
);

const TeamMember = entity(
  "TeamMember",
  {
    orgId: field.id("Organization"),
    teamId: field.id("Team"),
    userId: field.id("User"),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_team_user", fields: ["teamId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// Cycle — time-boxed sprint, team-scoped. Issues can be scheduled into a
// cycle; the active cycle is the one whose window contains now().
const Cycle = entity(
  "Cycle",
  {
    orgId: field.id("Organization"),
    teamId: field.id("Team"),
    number: field.float(),
    name: field.string().optional(),
    startsAt: field.datetime(),
    endsAt: field.datetime(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_team_number", fields: ["teamId", "number"], unique: true },
    ],
  },
);

const Project = entity(
  "Project",
  {
    orgId: field.id("Organization"),
    teamId: field.id("Team"),
    name: field.string(),
    description: field.string().optional(),
    status: field.string(), // planned | in_progress | paused | completed | cancelled
    leadId: field.id("User").optional(),
    targetDate: field.datetime().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_team_status", fields: ["teamId", "status"], unique: false },
    ],
  },
);

const Label = entity(
  "Label",
  {
    orgId: field.id("Organization"),
    teamId: field.id("Team"),
    name: field.string(),
    color: field.string(), // hex
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_team_name", fields: ["teamId", "name"], unique: true },
    ],
  },
);

// Core Issue entity. State machine mirrors Linear:
//   backlog → todo → in_progress → in_review → done
//   (+ cancelled, triage at the top for unassigned/new)
// Priority: 0 none | 1 urgent | 2 high | 3 medium | 4 low.
const Issue = entity(
  "Issue",
  {
    orgId: field.id("Organization"),
    teamId: field.id("Team"),
    number: field.float(), // monotonic per team; display key is `${team.key}-${number}`
    title: field.string(),
    description: field.string().optional(),
    state: field.string(),
    priority: field.float(),
    assigneeId: field.id("User").optional(),
    creatorId: field.id("User"),
    cycleId: field.id("Cycle").optional(),
    projectId: field.id("Project").optional(),
    estimate: field.float().optional(), // story points
    createdAt: field.datetime(),
    updatedAt: field.datetime(),
    startedAt: field.datetime().optional(),
    completedAt: field.datetime().optional(),
    cancelledAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_team_state", fields: ["teamId", "state"], unique: false },
      { name: "by_team_number", fields: ["teamId", "number"], unique: true },
      { name: "by_assignee", fields: ["assigneeId"], unique: false },
      { name: "by_cycle", fields: ["cycleId"], unique: false },
      { name: "by_project", fields: ["projectId"], unique: false },
    ],
  },
);

// Label attachments — join table so one issue can carry many labels.
const IssueLabel = entity(
  "IssueLabel",
  {
    orgId: field.id("Organization"),
    issueId: field.id("Issue"),
    labelId: field.id("Label"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_issue_label", fields: ["issueId", "labelId"], unique: true },
      { name: "by_label", fields: ["labelId"], unique: false },
    ],
  },
);

const Comment = entity(
  "Comment",
  {
    orgId: field.id("Organization"),
    issueId: field.id("Issue"),
    authorId: field.id("User"),
    body: field.string(),
    createdAt: field.datetime(),
    editedAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_issue", fields: ["issueId", "createdAt"], unique: false },
    ],
  },
);

const IssueActivity = entity(
  "IssueActivity",
  {
    orgId: field.id("Organization"),
    issueId: field.id("Issue"),
    actorId: field.id("User"),
    kind: field.string(), // state_changed | assigned | priority_changed | labeled | estimated | commented
    metaJson: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_issue", fields: ["issueId", "createdAt"], unique: false }],
  },
);

// ---------------------------------------------------------------------------
// Policies
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
  name: "linear",
  version: "0.1.0",
  entities: [
    User,
    Organization,
    OrgMember,
    Team,
    TeamMember,
    Cycle,
    Project,
    Label,
    Issue,
    IssueLabel,
    Comment,
    IssueActivity,
  ],
  queries: [],
  actions: [],
  policies: [
    organizationPolicy,
    orgMemberPolicy,
    orgScoped("Team"),
    orgScoped("TeamMember"),
    orgScoped("Cycle"),
    orgScoped("Project"),
    orgScoped("Label"),
    orgScoped("Issue"),
    orgScoped("IssueLabel"),
    orgScoped("Comment"),
    orgScoped("IssueActivity"),
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
